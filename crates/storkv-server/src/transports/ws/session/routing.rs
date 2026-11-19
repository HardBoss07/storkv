//! WebSocket upgrade routing and the socket frame loop.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::get;
use storkv_proto::MAX_FRAME_SIZE;
use tokio::sync::mpsc;

use super::{Outbound, Session, WATCH_CHANNEL_CAPACITY};
use crate::server::Server;
use crate::transports::http::HttpState;
use crate::transports::{ConnectionGuard, TransportKind, TransportOptions};

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

pub(super) async fn send_outbound(
    socket: &mut WebSocket,
    outbound: Outbound,
) -> Result<(), axum::Error> {
    let message = match outbound {
        Outbound::Binary(bytes) => WsMessage::Binary(bytes.into()),
        Outbound::Text(text) => WsMessage::Text(text.into()),
    };
    socket.send(message).await
}
