//! WebSocket session dispatch between binary and JSON-RPC frames.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bytes::BytesMut;
use storkv_proto::{Message, MessagePayload, Request, Response};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::json_rpc;
use crate::server::ConnectionConsistency;
use crate::transports::http::HttpState;

mod routing;
mod watch;

use watch::{send_watch_error, start_watcher};

const WATCH_CHANNEL_CAPACITY: usize = 128;
const WATCH_POLL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_WATCHERS_PER_SESSION: usize = 16;

pub use routing::standalone_router;
pub(crate) use routing::upgrade;

/// Queued outbound frame: binary `STORKPRT` frames for the native clients and
/// text frames for JSON tooling.
enum Outbound {
    Binary(Vec<u8>),
    Text(String),
}

/// A running watch subscription owned by one WebSocket session.
struct Watcher {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl Watcher {
    fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
        self.handle.abort();
    }
}

impl Outbound {
    fn acknowledge(request_id: u64) -> Self {
        let frame = Message::new_response(request_id, Response::Empty).encode();
        Self::Binary(frame.to_vec())
    }
}

struct Session {
    state: HttpState,
    is_admin: bool,
    consistency: ConnectionConsistency,
    sender: mpsc::Sender<Outbound>,
    watchers: Vec<Watcher>,
}

impl Session {
    fn new(state: HttpState, is_admin: bool, sender: mpsc::Sender<Outbound>) -> Self {
        let consistency = ConnectionConsistency::new(state.server.consistency());
        Self {
            state,
            is_admin,
            consistency,
            sender,
            watchers: Vec::new(),
        }
    }

    async fn handle_binary(
        &mut self,
        socket: &mut axum::extract::ws::WebSocket,
        bytes: &[u8],
    ) -> bool {
        let mut buffer = BytesMut::from(bytes);
        loop {
            match Message::decode(&mut buffer) {
                Ok(Some(message)) => {
                    if !self.handle_message(socket, message).await {
                        return false;
                    }
                }
                Ok(None) => return true,
                Err(error) => {
                    let response = Message::new_response(0, Response::Error(error));
                    return routing::send_outbound(
                        socket,
                        Outbound::Binary(response.encode().to_vec()),
                    )
                    .await
                    .is_ok();
                }
            }
        }
    }

    async fn handle_message(
        &mut self,
        socket: &mut axum::extract::ws::WebSocket,
        message: Message,
    ) -> bool {
        let MessagePayload::Request(request) = message.payload else {
            return true;
        };
        if matches!(request, Request::WatchSubscribe) {
            return self
                .start_watch(
                    socket,
                    message.request_id,
                    None,
                    Outbound::acknowledge,
                    false,
                )
                .await;
        }
        if let Request::WatchFrom { from_seq } = request {
            return self
                .start_watch(
                    socket,
                    message.request_id,
                    Some(from_seq),
                    Outbound::acknowledge,
                    false,
                )
                .await;
        }
        let response =
            self.state
                .server
                .dispatch_request(request, self.is_admin, &self.consistency);
        let frame = Message::new_response(message.request_id, response).encode();
        routing::send_outbound(socket, Outbound::Binary(frame.to_vec()))
            .await
            .is_ok()
    }

    async fn handle_text(&mut self, socket: &mut axum::extract::ws::WebSocket, text: &str) -> bool {
        let parsed = match json_rpc::parse(&self.state, text) {
            Ok(parsed) => parsed,
            Err(error) => {
                let response = json_rpc::error(error.id, "invalid_json", &error.message);
                return routing::send_outbound(socket, Outbound::Text(response))
                    .await
                    .is_ok();
            }
        };
        if matches!(parsed.request, Request::WatchSubscribe) {
            return self
                .start_watch(
                    socket,
                    parsed.id,
                    None,
                    |id| Outbound::Text(json_rpc::watching(id)),
                    true,
                )
                .await;
        }
        if let Request::WatchFrom { from_seq } = parsed.request {
            return self
                .start_watch(
                    socket,
                    parsed.id,
                    Some(from_seq),
                    |id| Outbound::Text(json_rpc::watching(id)),
                    true,
                )
                .await;
        }
        let response =
            self.state
                .server
                .dispatch_request(parsed.request, self.is_admin, &self.consistency);
        let text = json_rpc::success(parsed.id, response);
        routing::send_outbound(socket, Outbound::Text(text))
            .await
            .is_ok()
    }

    async fn start_watch<F>(
        &mut self,
        socket: &mut axum::extract::ws::WebSocket,
        request_id: u64,
        from_seq: Option<u64>,
        acknowledge: F,
        text: bool,
    ) -> bool
    where
        F: FnOnce(u64) -> Outbound,
    {
        self.watchers
            .retain(|watcher| !watcher.handle.is_finished());
        if self.watchers.len() >= MAX_WATCHERS_PER_SESSION {
            let error = storkv_proto::StorkvWireError::TooManySubscribers {
                count: self.watchers.len(),
                max: MAX_WATCHERS_PER_SESSION,
            };
            return send_watch_error(socket, request_id, error, text).await;
        }
        match start_watcher(
            self.state.server.clone(),
            request_id,
            from_seq,
            self.sender.clone(),
        ) {
            Ok(watcher) => self.watchers.push(watcher),
            Err(error) => {
                let wire = crate::dispatch::core_error_to_wire(error);
                return send_watch_error(socket, request_id, wire, text).await;
            }
        }
        routing::send_outbound(socket, acknowledge(request_id))
            .await
            .is_ok()
    }
}
