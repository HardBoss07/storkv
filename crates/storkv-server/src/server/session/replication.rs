//! Replication stream task for one native protocol session.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossbeam_channel::RecvTimeoutError;
use storkv_core::StorkvError;
use storkv_proto::{Message, Response};
use tokio::io::AsyncWrite;

use super::{ConnectionSession, stop_task};
use crate::ReplicationRole;

impl<W: AsyncWrite + Unpin> ConnectionSession<W> {
    pub(super) async fn start_replication(
        &mut self,
        request_id: u64,
        from_seq: u64,
    ) -> Result<(), std::io::Error> {
        if !self.is_admin && self.server.strict_admin() {
            let response = Response::Error(storkv_proto::StorkvWireError::Unauthorized);
            return self
                .write_message(Message::new_response(request_id, response))
                .await;
        }
        if self.server.role() == ReplicationRole::Replica {
            let response = Response::Error(storkv_proto::StorkvWireError::NotPrimary);
            return self
                .write_message(Message::new_response(request_id, response))
                .await;
        }
        stop_task(&mut self.replication_task, &mut self.replication_stop);
        let (frames, snapshot) = match self.server.db.replication_frames_from(from_seq) {
            Ok(frames) => (frames, None),
            Err(StorkvError::WatchCursorExpired { .. }) => {
                match self.server.db.replication_snapshot().and_then(
                    |(snapshot_seq, scopes, entries)| {
                        self.server
                            .db
                            .replication_frames_from(snapshot_seq)
                            .map(|frames| (frames, Some((snapshot_seq, scopes, entries))))
                    },
                ) {
                    Ok(result) => result,
                    Err(error) => {
                        let wire = crate::dispatch::core_error_to_wire(error);
                        return self
                            .write_message(Message::new_response(request_id, Response::Error(wire)))
                            .await;
                    }
                }
            }
            Err(error) => {
                let wire = crate::dispatch::core_error_to_wire(error);
                return self
                    .write_message(Message::new_response(request_id, Response::Error(wire)))
                    .await;
            }
        };

        // Complete the RPC handshake first so the client receives Response::Empty
        self.write_message(Message::new_response(request_id, Response::Empty))
            .await?;

        if let Some((snapshot_seq, scopes, entries)) = snapshot {
            self.write_message(Message::new_response(
                request_id,
                Response::ReplicationSnapshot {
                    snapshot_seq,
                    scopes,
                    entries,
                },
            ))
            .await?;
        }

        let sender = self.watch_sender.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let task_stop = stop.clone();
        self.replication_stop = Some(stop);
        self.replication_task = Some(tokio::task::spawn_blocking(move || {
            while !task_stop.load(Ordering::Relaxed) {
                let frame = match frames.recv_timeout(Duration::from_millis(250)) {
                    Ok(frame) => frame,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                if task_stop.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(encoded) = frame.encode(false) else {
                    break;
                };
                let response = Message::new_response(
                    request_id,
                    Response::ReplicationFrame {
                        sent_at_ms: super::super::now_millis(),
                        frame: bytes::Bytes::from(encoded),
                    },
                );
                if sender.try_send(response).is_err() {
                    break;
                }
            }
        }));
        Ok(())
    }
}
