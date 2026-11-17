//! WebSocket session loop, outbound queue, and watch subscriptions.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::get;
use bytes::BytesMut;
use storkv_proto::{MAX_FRAME_SIZE, Message, MessagePayload, Request, Response, WireChangeEvent};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::json_rpc;
use crate::server::Server;
use crate::transports::http::HttpState;
use crate::transports::{ConnectionGuard, TransportKind, TransportOptions};

const WATCH_CHANNEL_CAPACITY: usize = 128;
const WATCH_POLL_INTERVAL: Duration = Duration::from_millis(250);

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
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
        self.handle.abort();
    }
}

/// Upgrade a request on the shared HTTP listener to a WebSocket session.
pub(crate) async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<HttpState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let is_admin = state.is_admin(&headers);
    let server = state.server.clone();
    ws.max_message_size(MAX_FRAME_SIZE)
        .on_upgrade(move |socket| async move {
            let _guard = ConnectionGuard::new(server.clone(), TransportKind::WebSocket);
            handle_socket(socket, state, is_admin).await;
        })
}

/// Build the router for a dedicated `--ws-listen` listener.
pub fn standalone_router(server: Arc<Server>, options: TransportOptions) -> Router {
    let state = HttpState { server, options };
    Router::new()
        .route("/v1/ws", get(upgrade))
        .with_state(state)
}

struct Session {
    state: HttpState,
    is_admin: bool,
    sender: mpsc::Sender<Outbound>,
    watchers: Vec<Watcher>,
}

impl Session {
    fn new(state: HttpState, is_admin: bool, sender: mpsc::Sender<Outbound>) -> Self {
        Self {
            state,
            is_admin,
            sender,
            watchers: Vec::new(),
        }
    }

    async fn handle_binary(&mut self, socket: &mut WebSocket, bytes: &[u8]) -> bool {
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
                    return send_outbound(socket, Outbound::Binary(response.encode().to_vec()))
                        .await
                        .is_ok();
                }
            }
        }
    }

    async fn handle_message(&mut self, socket: &mut WebSocket, message: Message) -> bool {
        let MessagePayload::Request(request) = message.payload else {
            return true;
        };
        if matches!(request, Request::WatchSubscribe) {
            return self
                .start_watch(socket, message.request_id, Outbound::acknowledge)
                .await;
        }
        let response = self.state.server.dispatch_request(request, self.is_admin);
        let frame = Message::new_response(message.request_id, response).encode();
        send_outbound(socket, Outbound::Binary(frame.to_vec()))
            .await
            .is_ok()
    }

    async fn handle_text(&mut self, socket: &mut WebSocket, text: &str) -> bool {
        let parsed = match json_rpc::parse(&self.state, text) {
            Ok(parsed) => parsed,
            Err(error) => {
                let response = json_rpc::error(error.id, "invalid_json", &error.message);
                return send_outbound(socket, Outbound::Text(response))
                    .await
                    .is_ok();
            }
        };
        if matches!(parsed.request, Request::WatchSubscribe) {
            return self
                .start_watch(socket, parsed.id, |id| {
                    Outbound::Text(json_rpc::watching(id))
                })
                .await;
        }
        let response = self
            .state
            .server
            .dispatch_request(parsed.request, self.is_admin);
        let text = json_rpc::success(parsed.id, response);
        send_outbound(socket, Outbound::Text(text)).await.is_ok()
    }

    async fn start_watch<F>(
        &mut self,
        socket: &mut WebSocket,
        request_id: u64,
        acknowledge: F,
    ) -> bool
    where
        F: FnOnce(u64) -> Outbound,
    {
        self.watchers.push(start_watcher(
            self.state.server.clone(),
            request_id,
            self.sender.clone(),
        ));
        send_outbound(socket, acknowledge(request_id)).await.is_ok()
    }
}

impl Outbound {
    fn acknowledge(request_id: u64) -> Self {
        let frame = Message::new_response(request_id, Response::Empty).encode();
        Self::Binary(frame.to_vec())
    }
}

async fn handle_socket(mut socket: WebSocket, state: HttpState, is_admin: bool) {
    let (sender, mut receiver) = mpsc::channel::<Outbound>(WATCH_CHANNEL_CAPACITY);
    let mut session = Session::new(state.clone(), is_admin, sender);
    let mut shutdown = state.server.shutdown_sender.subscribe();

    loop {
        tokio::select! {
            frame = socket.recv() => {
                if !handle_frame(&mut session, &mut socket, frame).await {
                    break;
                }
            }
            Some(outbound) = receiver.recv() => {
                if send_outbound(&mut socket, outbound).await.is_err() {
                    break;
                }
            }
            _ = shutdown.recv() => break,
        }
    }
    for watcher in &session.watchers {
        watcher.stop();
    }
}

async fn handle_frame(
    session: &mut Session,
    socket: &mut WebSocket,
    frame: Option<Result<WsMessage, axum::Error>>,
) -> bool {
    match frame {
        Some(Ok(WsMessage::Binary(bytes))) => session.handle_binary(socket, &bytes).await,
        Some(Ok(WsMessage::Text(text))) => session.handle_text(socket, &text).await,
        Some(Ok(WsMessage::Close(_))) | None => false,
        Some(Ok(WsMessage::Ping(_) | WsMessage::Pong(_))) => true,
        Some(Err(_)) => false,
    }
}

async fn send_outbound(socket: &mut WebSocket, outbound: Outbound) -> Result<(), axum::Error> {
    let message = match outbound {
        Outbound::Binary(bytes) => WsMessage::Binary(bytes.into()),
        Outbound::Text(text) => WsMessage::Text(text.into()),
    };
    socket.send(message).await
}

fn start_watcher(server: Arc<Server>, request_id: u64, sender: mpsc::Sender<Outbound>) -> Watcher {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let handle = tokio::task::spawn_blocking(move || {
        let receiver = server.db.watch();
        while !stop_flag.load(Ordering::Relaxed) {
            match receiver.recv_timeout(WATCH_POLL_INTERVAL) {
                Ok(event) => {
                    let Some(event) = wire_change_event(event) else {
                        continue;
                    };
                    let frame = Message::new_response(request_id, Response::WatchEvent(event));
                    if sender
                        .blocking_send(Outbound::Binary(frame.encode().to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    Watcher { stop, handle }
}

fn wire_change_event(event: storkv_core::watch::ChangeEvent) -> Option<WireChangeEvent> {
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
        _ => None,
    }
}
