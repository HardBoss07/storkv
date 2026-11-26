//! etcd v3 HTTP/JSON gateway (`/v3/kv/*` and `/v3/lease/*`) backed by a
//! dedicated scope.
//!
//! Keys and values are base64, responses carry etcd's `header` object, and
//! `revision` is the engine's last visible sequence number.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response as HttpResponse};
use bytes::Bytes;
use serde_json::{Value, json};
use storkv_core::{ScopeHandle, TimestampMs};

use crate::compat;
use crate::transports::http::HttpState;
use crate::transports::http::json::{decode_base64, encode_base64};

mod kv;
mod lease;
mod lease_registry;
mod txn;

pub(crate) use kv::{delete_range, put, range};
pub(crate) use lease::{
    grant as lease_grant, keepalive as lease_keepalive, revoke as lease_revoke,
    time_to_live as lease_time_to_live,
};
pub(crate) use lease_registry::LeaseRegistry;
pub(crate) use txn::txn;

/// etcd gRPC status code for an invalid argument.
const CODE_INVALID_ARGUMENT: i32 = 3;
/// etcd gRPC status code for a failed precondition.
const CODE_FAILED_PRECONDITION: i32 = 9;
/// etcd gRPC status code for an internal error.
const CODE_INTERNAL: i32 = 13;
/// A one-byte range end of zero means "scan to the last key".
const RANGE_END_TO_LAST_KEY: [u8; 1] = [0];

fn require_primary(state: &HttpState) -> Result<(), Box<HttpResponse>> {
    state
        .server
        .ensure_primary()
        .map_err(|error| Box::new(write_error(error)))
}

fn write_error(error: storkv_core::StorkvError) -> HttpResponse {
    match error {
        storkv_core::StorkvError::NotPrimary => gateway_error(
            StatusCode::BAD_REQUEST,
            CODE_FAILED_PRECONDITION,
            "This node is a replica and cannot accept writes",
        ),
        storkv_core::StorkvError::InvalidOperation(message) => {
            gateway_error(StatusCode::BAD_REQUEST, CODE_INVALID_ARGUMENT, &message)
        }
        other => gateway_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            CODE_INTERNAL,
            &other.to_string(),
        ),
    }
}

fn scope(state: &HttpState) -> Result<ScopeHandle, Box<HttpResponse>> {
    compat::etcd_scope(state.server.db(), state.server.is_primary()).map_err(|error| {
        Box::new(gateway_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            CODE_INTERNAL,
            &format!("scope unavailable: {error}"),
        ))
    })
}

/// Read one key or one ordered key range with an optional limit.
fn scan_range(
    state: &HttpState,
    scope: &ScopeHandle,
    key: &[u8],
    range_end: &[u8],
    limit: Option<usize>,
    keys_only: bool,
) -> storkv_core::Result<Vec<(Bytes, Bytes)>> {
    let db = state.server.db();
    if range_end.is_empty() {
        return Ok(db
            .get(scope, key)?
            .map(|value| {
                let value = if keys_only { Bytes::new() } else { value };
                vec![(Bytes::copy_from_slice(key), value)]
            })
            .unwrap_or_default());
    }
    let end = if range_end == RANGE_END_TO_LAST_KEY {
        None
    } else {
        Some(range_end)
    };
    db.scan_with(
        scope,
        Some(key),
        end,
        storkv_core::ScanOptions {
            limit,
            keys_only,
            ..Default::default()
        },
    )
}

/// Build one etcd key/value entry for a range response.
fn entry_json(key: &[u8], value: Option<&[u8]>, version: u64, lease: u64) -> Value {
    let mut entry = json!({
        "key": encode_base64(key),
        "version": version.to_string(),
        "create_revision": version.to_string(),
        "mod_revision": version.to_string(),
        "lease": lease.to_string(),
    });
    if let (Some(value), Some(object)) = (value, entry.as_object_mut()) {
        object.insert("value".into(), Value::String(encode_base64(value)));
    }
    entry
}

fn revision(state: &HttpState, _scope_id: u32, mut payload: Value) -> Json<Value> {
    let revision = state
        .server
        .db()
        .inner()
        .last_visible_seq
        .load(std::sync::atomic::Ordering::Acquire);
    let header = json!({
        "cluster_id": "53544f524b56",
        "member_id": "73746f726b76",
        "revision": revision.to_string(),
        "raft_term": "1",
    });
    if let Some(object) = payload.as_object_mut() {
        object.insert("header".into(), header);
    }
    Json(payload)
}

fn field(body: &Value, name: &str) -> Option<Vec<u8>> {
    body.get(name)
        .and_then(Value::as_str)
        .and_then(decode_base64)
}

/// Read an integer field that a client sent as a JSON number or a decimal string.
fn field_number(body: &Value, name: &str) -> Option<u64> {
    match body.get(name)? {
        Value::String(text) => text.parse().ok(),
        Value::Number(number) => number.as_u64(),
        _ => None,
    }
}

fn now_ms() -> u64 {
    TimestampMs::now().get()
}

fn gateway_error(status: StatusCode, code: i32, message: &str) -> HttpResponse {
    (
        status,
        Json(json!({
            "error": message,
            "message": message,
            "code": code,
        })),
    )
        .into_response()
}
