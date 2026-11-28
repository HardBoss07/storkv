//! Per-connection native protocol session.
//!
//! One session owns the connection writer, the watch and replication task
//! handles, and the per-connection consistency level. The tasks themselves
//! live in the child modules.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use storkv_proto::{Message, MessagePayload, Request, Response};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::task::JoinHandle;

use super::{ConnectionConsistency, Server};
use crate::transports::TransportKind;

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

    pub(super) async fn write_message(&mut self, message: Message) -> Result<(), std::io::Error> {
        self.writer.write_all(&message.encode()).await
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

mod replication;
mod watch;
