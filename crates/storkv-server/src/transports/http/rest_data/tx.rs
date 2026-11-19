//! REST handlers for explicit transactions.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response as HttpResponse;
use serde_json::Value;
use storkv_proto::{Request, WireTxMutation};

use super::super::HttpState;
use super::super::json::{json_error, response_payload};
use super::support::field_bytes;

pub(crate) async fn tx_begin(State(state): State<HttpState>, headers: HeaderMap) -> HttpResponse {
    response_payload(state.dispatch(Request::TxBegin, &headers))
}

pub(crate) async fn tx_commit(
    State(state): State<HttpState>,
    headers: HeaderMap,
    body: Bytes,
) -> HttpResponse {
    match parse_commit_body(&state, &body) {
        Some(request) => response_payload(state.dispatch(request, &headers)),
        None => json_error(
            StatusCode::BAD_REQUEST,
            "invalid_body",
            "invalid transaction payload",
        ),
    }
}

fn parse_commit_body(state: &HttpState, body: &[u8]) -> Option<Request> {
    let parsed = serde_json::from_slice::<Value>(body).ok()?;
    let read_seq = parsed.get("read_seq").and_then(Value::as_u64)?;
    let mut read_set = Vec::new();
    for entry in parsed.get("read_set")?.as_array()? {
        let scope_id = resolve_scope_value(state, entry)?;
        let key = field_bytes(entry, "key_b64")?;
        read_set.push((scope_id, key));
    }
    let mut write_set = Vec::new();
    for entry in parsed.get("write_set")?.as_array()? {
        let scope_id = resolve_scope_value(state, entry)?;
        let key = field_bytes(entry, "key_b64")?;
        let mutation = parse_mutation(entry)?;
        write_set.push(((scope_id, key), mutation));
    }
    Some(Request::TxCommit {
        read_seq,
        read_set,
        write_set,
    })
}

fn resolve_scope_value(state: &HttpState, entry: &Value) -> Option<u32> {
    if let Some(scope_id) = entry.get("scope_id").and_then(Value::as_u64) {
        return u32::try_from(scope_id).ok();
    }
    let name = entry.get("scope").and_then(Value::as_str)?;
    state
        .server
        .db()
        .inner()
        .scope_names
        .read()
        .get(name)
        .copied()
}

fn parse_mutation(entry: &Value) -> Option<WireTxMutation> {
    match entry.get("kind").and_then(Value::as_str).unwrap_or("put") {
        "delete" => Some(WireTxMutation::Delete),
        "delete_range" => Some(WireTxMutation::DeleteRange {
            end_key: field_bytes(entry, "end_b64")?,
        }),
        "merge" => Some(WireTxMutation::Merge {
            operand: field_bytes(entry, "operand_b64")?,
        }),
        _ => Some(WireTxMutation::Put {
            val: field_bytes(entry, "value_b64").unwrap_or_default(),
            expires_at_ms: entry
                .get("expires_at_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        }),
    }
}
