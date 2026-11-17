//! JSON-RPC endpoint that accepts a serialized [`storkv_proto::Request`].

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response as HttpResponse};
use bytes::BytesMut;
use storkv_proto::{Message, MessagePayload};

use super::HttpState;
use super::json::json_error;
use crate::transports::TransportKind;

/// Binary native-frame tunnel used by the unified client's HTTP backend.
///
/// The body is one `STORKPRT` v1 request frame and the response is one
/// `STORKPRT` v1 response frame, byte-for-byte identical to the native TCP
/// transport. The human-facing REST routes remain the documented public API.
pub(super) async fn handle(
    State(state): State<HttpState>,
    headers: HeaderMap,
    body: Bytes,
) -> HttpResponse {
    state
        .server
        .stats
        .record_transport_request(TransportKind::Http.as_u8());
    let mut buffer = BytesMut::from(body.as_ref());
    match Message::decode(&mut buffer) {
        Ok(Some(Message {
            request_id,
            payload: MessagePayload::Request(request),
        })) => {
            let is_admin = state.is_admin(&headers);
            let response = state.server.dispatch_request(request, is_admin);
            let encoded = Message::new_response(request_id, response).encode();
            (
                StatusCode::OK,
                [("content-type", "application/octet-stream")],
                encoded,
            )
                .into_response()
        }
        Ok(Some(_)) => json_error(
            StatusCode::BAD_REQUEST,
            "invalid_frame",
            "expected a request frame",
        ),
        Ok(None) => json_error(
            StatusCode::BAD_REQUEST,
            "truncated_frame",
            "incomplete request frame",
        ),
        Err(error) => json_error(StatusCode::BAD_REQUEST, "invalid_frame", &error.to_string()),
    }
}
