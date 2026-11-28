//! Watch subscription task for one native protocol session.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossbeam_channel::RecvTimeoutError;
use storkv_proto::{Message, Response, WireChangeEvent};
use tokio::io::AsyncWrite;

use super::{ConnectionSession, stop_task};

impl<W: AsyncWrite + Unpin> ConnectionSession<W> {
    pub(super) async fn start_watch(
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

    pub(crate) fn stop_watch(&mut self) {
        stop_task(&mut self.watch_task, &mut self.watch_stop);
        stop_task(&mut self.replication_task, &mut self.replication_stop);
    }
}

fn wire_change_event(event: storkv_core::watch::ChangeEvent) -> Option<WireChangeEvent> {
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
