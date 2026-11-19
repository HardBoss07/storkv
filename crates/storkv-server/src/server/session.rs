//! Per-connection native protocol session.

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
    pub(super) replication_task: Option<JoinHandle<()>>,
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
        if let Some(handle) = self.watch_task.take() {
            handle.abort();
        }
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
        self.watch_task = Some(tokio::task::spawn_blocking(move || {
            while let Ok(event) = watch_receiver.recv() {
                let Some(wire_event) = wire_change_event(event) else {
                    continue;
                };
                let response = Message::new_response(request_id, Response::WatchEvent(wire_event));
                if watch_sender.blocking_send(response).is_err() {
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
        if let Some(handle) = self.replication_task.take() {
            handle.abort();
        }
        let (frames, snapshot) = match self.server.db.replication_frames_from(from_seq) {
            Ok(frames) => (frames, None),
            Err(StorkvError::WatchCursorExpired {
                oldest_available, ..
            }) => {
                match self.server.db.replication_snapshot().and_then(|entries| {
                    self.server
                        .db
                        .replication_frames_from(oldest_available)
                        .map(|frames| (entries, frames))
                }) {
                    Ok((entries, frames)) => (frames, Some(entries)),
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
        if let Some(entries) = snapshot {
            self.write_message(Message::new_response(
                request_id,
                Response::ReplicationSnapshot(entries),
            ))
            .await?;
        }
        let sender = self.watch_sender.clone();
        self.replication_task = Some(tokio::task::spawn_blocking(move || {
            while let Ok(frame) = frames.recv() {
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
                if sender.blocking_send(response).is_err() {
                    break;
                }
            }
        }));
        self.write_message(Message::new_response(request_id, Response::Empty))
            .await
    }

    pub(super) fn stop_watch(&mut self) {
        if let Some(handle) = self.watch_task.take() {
            handle.abort();
        }
        if let Some(handle) = self.replication_task.take() {
            handle.abort();
        }
    }
}

pub(super) fn wire_change_event(event: storkv_core::watch::ChangeEvent) -> Option<WireChangeEvent> {
    match event {
        storkv_core::watch::ChangeEvent::Put {
            scope_id,
            key,
            value,
            seq,
        } => Some(WireChangeEvent::Put {
            scope_id,
            key,
            value,
            seq,
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
        _ => None,
    }
}
