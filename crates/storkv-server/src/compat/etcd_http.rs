//! etcd v3 HTTP/JSON gateway (`/v3/kv/*`) backed by a dedicated scope.
//!
//! Keys and values are base64, responses carry etcd's `header` object, and
//! `revision` is the engine's last visible sequence number.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response as HttpResponse};
use bytes::Bytes;
use serde_json::{Value, json};
use storkv_core::ScopeHandle;

use crate::compat;
use crate::transports::http::HttpState;
use crate::transports::http::json::{decode_base64, encode_base64};

/// etcd gRPC status code for an invalid argument.
const CODE_INVALID_ARGUMENT: i32 = 3;
/// etcd gRPC status code for a failed precondition.
const CODE_FAILED_PRECONDITION: i32 = 9;
/// etcd gRPC status code for an internal error.
const CODE_INTERNAL: i32 = 13;
/// A one-byte range end of zero means "scan to the last key".
const RANGE_END_TO_LAST_KEY: [u8; 1] = [0];

pub(crate) async fn put(State(state): State<HttpState>, Json(body): Json<Value>) -> HttpResponse {
    if let Err(response) = require_primary(&state) {
        return *response;
    }
    let scope = match scope(&state) {
        Ok(scope) => scope,
        Err(response) => return *response,
    };
    let lease = body.get("lease").and_then(Value::as_u64).unwrap_or(0);
    if lease != 0 {
        return gateway_error(
            StatusCode::BAD_REQUEST,
            CODE_INVALID_ARGUMENT,
            "leases are not supported",
        );
    }
    let Some(key) = field(&body, "key") else {
        return gateway_error(
            StatusCode::BAD_REQUEST,
            CODE_INVALID_ARGUMENT,
            "key is required",
        );
    };
    let value = field(&body, "value").unwrap_or_default();
    if let Err(error) = crate::compat::redis::keyspace::put_absolute(
        state.server.db(),
        state.server.is_primary(),
        &scope,
        &key,
        &value,
        0,
    ) {
        return write_error(error);
    }
    revision(&state, scope.scope_id, json!({})).into_response()
}

pub(crate) async fn range(State(state): State<HttpState>, Json(body): Json<Value>) -> HttpResponse {
    let scope = match scope(&state) {
        Ok(scope) => scope,
        Err(response) => return *response,
    };
    let Some(key) = field(&body, "key") else {
        return gateway_error(
            StatusCode::BAD_REQUEST,
            CODE_INVALID_ARGUMENT,
            "key is required",
        );
    };
    let range_end = field(&body, "range_end").unwrap_or_default();
    let limit = body
        .get("limit")
        .and_then(Value::as_u64)
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(0);
    let count_only = body
        .get("count_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let keys_only = body
        .get("keys_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let pairs = match scan_range(
        &state,
        &scope,
        &key,
        &range_end,
        if limit == 0 { None } else { Some(limit) },
        keys_only,
    ) {
        Ok(pairs) => pairs,
        Err(error) => {
            return gateway_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                CODE_INTERNAL,
                &error.to_string(),
            );
        }
    };
    let count = pairs.len();
    let mut kvs = Vec::new();
    if !count_only {
        for (key, value) in pairs {
            let meta = state.server.db().key_meta(&scope, &key).ok().flatten();
            let version = meta.map(|meta| meta.seq).unwrap_or(0);
            let mut entry = json!({
                "key": encode_base64(&key),
                "version": version.to_string(),
                "create_revision": version.to_string(),
                "mod_revision": version.to_string(),
                "lease": "0",
            });
            if !keys_only {
                if let Some(object) = entry.as_object_mut() {
                    object.insert("value".into(), Value::String(encode_base64(&value)));
                }
            }
            kvs.push(entry);
        }
    }
    revision(
        &state,
        scope.scope_id,
        json!({"kvs": kvs, "count": count.to_string(), "more": false}),
    )
    .into_response()
}

pub(crate) async fn delete_range(
    State(state): State<HttpState>,
    Json(body): Json<Value>,
) -> HttpResponse {
    if let Err(response) = require_primary(&state) {
        return *response;
    }
    let scope = match scope(&state) {
        Ok(scope) => scope,
        Err(response) => return *response,
    };
    let Some(key) = field(&body, "key") else {
        return gateway_error(
            StatusCode::BAD_REQUEST,
            CODE_INVALID_ARGUMENT,
            "key is required",
        );
    };
    let range_end = field(&body, "range_end").unwrap_or_default();
    let pairs = match scan_range(&state, &scope, &key, &range_end, None, true) {
        Ok(pairs) => pairs,
        Err(error) => {
            return gateway_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                CODE_INTERNAL,
                &error.to_string(),
            );
        }
    };
    let deleted = pairs.len();
    let db = state.server.db();
    let mut transaction = db.begin_tx();
    for (key, _) in &pairs {
        if let Err(error) = transaction.delete(&scope, key) {
            return write_error(error);
        }
    }
    if let Err(error) = transaction.commit() {
        return write_error(error);
    }
    revision(
        &state,
        scope.scope_id,
        json!({"deleted": deleted.to_string()}),
    )
    .into_response()
}

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
