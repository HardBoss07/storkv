//! Watch subscriptions for one WebSocket session.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::ws::WebSocket;
use storkv_proto::{Message, Response};
use tokio::sync::mpsc;

use super::super::json_rpc;
use super::{Outbound, WATCH_POLL_INTERVAL, Watcher, wire_change_event};
use crate::server::Server;
use crate::transports::http::json::code_for_wire_error;

pub(super) async fn send_watch_error(
    socket: &mut WebSocket,
    request_id: u64,
    error: storkv_proto::StorkvWireError,
    text: bool,
) -> bool {
    let outbound = if text {
        Outbound::Text(json_rpc::error(
            request_id,
            code_for_wire_error(&error),
            &error.to_string(),
        ))
    } else {
        let frame = Message::new_response(request_id, Response::Error(error)).encode();
        Outbound::Binary(frame.to_vec())
    };
    super::routing::send_outbound(socket, outbound)
        .await
        .is_ok()
}

pub(super) fn start_watcher(
    server: Arc<Server>,
    request_id: u64,
    from_seq: Option<u64>,
    sender: mpsc::Sender<Outbound>,
) -> Result<Watcher, storkv_core::StorkvError> {
    let receiver = match from_seq {
        Some(seq) => server.db.watch_from(seq)?,
        None => server.db.watch()?,
    };
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let handle = tokio::task::spawn_blocking(move || {
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
    Ok(Watcher { stop, handle })
}
