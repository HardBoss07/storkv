//! Per-connection native protocol session.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossbeam_channel::RecvTimeoutError;
use storkv_proto::{Message, MessagePayload, Request, Response, WireChangeEvent};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::task::JoinHandle;

use super::{ConnectionConsistency, Server};
use crate::ReplicationRole;
use crate::transports::TransportKind;
use storkv_core::StorkvError;

pub(super) struct ConnectionSession<W> {
    pub(super) server: std::sync::Arc<Server>,
    pub(super) writer: W,
    pub(super) watch_sender: tokio::sync::mpsc::Sender<Message>,
    pub(super) watch_task: Option<JoinHandle<()>>,
    pub(super) watch_stop: Option<Arc<AtomicBool>>,
    pub(super) replication_task: Option<JoinHandle<()>>,
    pub(super) replication_stop: Option<Arc<AtomicBool>>,
    pub(super) consistency: ConnectionConsistency,
    pub(super) is_admin: bool,
}

impl<W: AsyncWrite + Unpin> ConnectionSession<W> {
    pub(super) async fn process_buffered_messages(
        &mut self,
        read_buffer: &mut bytes::BytesMut,
    ) -> Result<bool, std::io::Error> {
        loop {
            let message = match Message::decode(read_buffer) {
                Ok(Some(message)) => message,
                Ok(None) => return Ok(true),
                Err(wire_error) => {
                    let error_response = Message::new_response(0, Response::Error(wire_error));
                    self.write_message(error_response).await?;
                    return Ok(false);
                }
            };
            self.handle_message(message).await?;
        }
    }

    async fn handle_message(&mut self, message: Message) -> Result<(), std::io::Error> {
        let MessagePayload::Request(request) = message.payload else {
            return Ok(());
        };
        self.server
            .stats
            .record_transport_request(TransportKind::Native.as_u8());
        if request == Request::WatchSubscribe {
            self.start_watch(message.request_id, None).await?;
        } else if let Request::WatchFrom { from_seq } = request {
            self.start_watch(message.request_id, Some(from_seq)).await?;
        } else if let Request::ReplicationStart { from_seq } = request {
            self.start_replication(message.request_id, from_seq).await?;
        } else {
            let response = self
                .server
                .dispatch_request(request, self.is_admin, &self.consistency);
            self.write_message(Message::new_response(message.request_id, response))
                .await?;
        }
        Ok(())
    }

    async fn start_watch(
        &mut self,
        request_id: u64,
        from_seq: Option<u64>,
    ) -> Result<(), std::io::Error> {
        stop_task(&mut self.watch_task, &mut self.watch_stop);
        let receiver = match from_seq {
            Some(seq) => self.server.db.watch_from(seq),
            None => self.server.db.watch(),
        };
        let watch_receiver = match receiver {
            Ok(receiver) => receiver,
            Err(error) => {
                let wire = crate::dispatch::core_error_to_wire(error);
                return self
                    .write_message(Message::new_response(request_id, Response::Error(wire)))
                    .await;
            }
        };
        let watch_sender = self.watch_sender.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let task_stop = stop.clone();
        self.watch_stop = Some(stop);
        self.watch_task = Some(tokio::task::spawn_blocking(move || {
            // `spawn_blocking` tasks cannot be aborted once they start, so the
            // task watches this flag and polls the receiver with a timeout.
            while !task_stop.load(Ordering::Relaxed) {
                let event = match watch_receiver.recv_timeout(Duration::from_millis(250)) {
                    Ok(event) => event,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                if task_stop.load(Ordering::Relaxed) {
                    break;
                }
                let Some(wire_event) = wire_change_event(event) else {
                    continue;
                };
                let response = Message::new_response(request_id, Response::WatchEvent(wire_event));
                if watch_sender.try_send(response).is_err() {
                    break;
                }
            }
        }));
        self.write_message(Message::new_response(request_id, Response::Empty))
            .await
    }

    pub(super) async fn write_message(&mut self, message: Message) -> Result<(), std::io::Error> {
        self.writer.write_all(&message.encode()).await
    }

    async fn start_replication(
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
                        sent_at_ms: super::now_millis(),
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

    pub(super) fn stop_watch(&mut self) {
        stop_task(&mut self.watch_task, &mut self.watch_stop);
        stop_task(&mut self.replication_task, &mut self.replication_stop);
    }
}

/// Signal a blocking task to stop, then abort it in case it has not started.
fn stop_task(handle: &mut Option<JoinHandle<()>>, stop: &mut Option<Arc<AtomicBool>>) {
    if let Some(flag) = stop.take() {
        flag.store(true, Ordering::Relaxed);
    }
    if let Some(handle) = handle.take() {
        handle.abort();
    }
}

pub(super) fn wire_change_event(event: storkv_core::watch::ChangeEvent) -> Option<WireChangeEvent> {
    match event {
        storkv_core::watch::ChangeEvent::Put {
            scope_id,
            key,
            value,
            seq,
            expires_at_ms,
        } => Some(WireChangeEvent::Put {
            scope_id,
            key,
            value,
            seq,
            expires_at_ms,
        }),
        storkv_core::watch::ChangeEvent::Delete { scope_id, key, seq } => {
            Some(WireChangeEvent::Delete { scope_id, key, seq })
        }
        storkv_core::watch::ChangeEvent::DropScope { scope_id, seq } => {
            Some(WireChangeEvent::DropScope { scope_id, seq })
        }
        storkv_core::watch::ChangeEvent::InvalidateScope { scope_id, seq } => {
            Some(WireChangeEvent::InvalidateScope { scope_id, seq })
        }
        storkv_core::watch::ChangeEvent::Expired { scope_id, key, seq } => {
            Some(WireChangeEvent::Expired { scope_id, key, seq })
        }
        _ => None,
    }
}
