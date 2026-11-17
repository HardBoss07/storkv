//! REST data-plane handlers: keys, scans, merges, and transactions.

use std::collections::HashMap;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response as HttpResponse};
use bytes::Bytes as ProtoBytes;
use serde_json::Value;
use storkv_proto::{Request, Response, WireMergeOp, WireTxMutation};

use super::HttpState;
use super::json::{decode_base64, json_error, response_payload};

pub(super) type ScopeQuery = Query<HashMap<String, String>>;

pub(super) async fn get_key(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path((scope, key)): Path<(String, String)>,
    Query(query): ScopeQuery,
) -> HttpResponse {
    let scope_id = match resolve_scope(&state, &scope) {
        Ok(scope_id) => scope_id,
        Err(response) => return *response,
    };
    let Some(key) = key_from(&key, &query) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_key", "invalid base64 key");
    };
    let response = state.dispatch(Request::Get { scope_id, key }, &headers);
    match response {
        Response::Value(Some(value)) if query.get("format").map(String::as_str) != Some("json") => {
            HttpResponse::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/octet-stream")
                .body(axum::body::Body::from(value))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Response::Value(None) => json_error(StatusCode::NOT_FOUND, "not_found", "key not found"),
        other => response_payload(other),
    }
}

pub(super) async fn put_key(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path((scope, key)): Path<(String, String)>,
    Query(query): ScopeQuery,
    body: Bytes,
) -> HttpResponse {
    let scope_id = match resolve_scope(&state, &scope) {
        Ok(scope_id) => scope_id,
        Err(response) => return *response,
    };
    let Some(key) = key_from(&key, &query) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_key", "invalid base64 key");
    };
    let Some(value) = value_from(&query, &body) else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "invalid_body",
            "invalid base64 value",
        );
    };
    let ttl = query
        .get("ttl_ms")
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|ttl| *ttl > 0);
    let request = match ttl {
        Some(ttl) => Request::PutWithTtl {
            scope_id,
            key,
            val: value,
            ttl: Duration::from_millis(ttl),
        },
        None => Request::Put {
            scope_id,
            key,
            val: value,
        },
    };
    response_payload(state.dispatch(request, &headers))
}

pub(super) async fn delete_key(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path((scope, key)): Path<(String, String)>,
    Query(query): ScopeQuery,
) -> HttpResponse {
    let scope_id = match resolve_scope(&state, &scope) {
        Ok(scope_id) => scope_id,
        Err(response) => return *response,
    };
    let Some(key) = key_from(&key, &query) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_key", "invalid base64 key");
    };
    response_payload(state.dispatch(Request::Delete { scope_id, key }, &headers))
}

pub(super) async fn merge(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path((scope, key)): Path<(String, String)>,
    Query(query): ScopeQuery,
    body: Bytes,
) -> HttpResponse {
    let scope_id = match resolve_scope(&state, &scope) {
        Ok(scope_id) => scope_id,
        Err(response) => return *response,
    };
    let Some(key) = key_from(&key, &query) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_key", "invalid base64 key");
    };
    let parsed = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    let op = match parsed.get("op").and_then(Value::as_str) {
        Some("custom") => match parsed.get("operand_b64").and_then(Value::as_str) {
            Some(encoded) => match decode_base64(encoded) {
                Some(bytes) => WireMergeOp::Custom(ProtoBytes::from(bytes)),
                None => {
                    return json_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_body",
                        "invalid operand_b64",
                    );
                }
            },
            None => {
                return json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_body",
                    "missing operand_b64",
                );
            }
        },
        _ => WireMergeOp::CounterAdd(parsed.get("delta").and_then(Value::as_i64).unwrap_or(1)),
    };
    response_payload(state.dispatch(Request::Merge { scope_id, key, op }, &headers))
}

pub(super) async fn delete_range(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path(scope): Path<String>,
    body: Bytes,
) -> HttpResponse {
    let scope_id = match resolve_scope(&state, &scope) {
        Ok(scope_id) => scope_id,
        Err(response) => return *response,
    };
    let parsed = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    let Some(start_key) = field_bytes(&parsed, "start_b64") else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_body", "missing start_b64");
    };
    let Some(end_key) = field_bytes(&parsed, "end_b64") else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_body", "missing end_b64");
    };
    response_payload(state.dispatch(
        Request::DeleteRange {
            scope_id,
            start_key,
            end_key,
        },
        &headers,
    ))
}

pub(super) async fn scan(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path(scope): Path<String>,
    Query(query): ScopeQuery,
) -> HttpResponse {
    let scope_id = match resolve_scope(&state, &scope) {
        Ok(scope_id) => scope_id,
        Err(response) => return *response,
    };
    if let Some(prefix) = query.get("prefix_b64").and_then(|raw| decode_base64(raw)) {
        return response_payload(state.dispatch(
            Request::ScanPrefix {
                scope_id,
                prefix: ProtoBytes::from(prefix),
            },
            &headers,
        ));
    }
    if let Some(prefix) = query.get("prefix").filter(|prefix| !prefix.is_empty()) {
        return response_payload(state.dispatch(
            Request::ScanPrefix {
                scope_id,
                prefix: ProtoBytes::from(prefix.as_bytes().to_vec()),
            },
            &headers,
        ));
    }
    let start_key = query
        .get("start_b64")
        .and_then(|raw| decode_base64(raw))
        .map(ProtoBytes::from)
        .or_else(|| {
            query
                .get("start")
                .filter(|raw| !raw.is_empty())
                .map(|raw| ProtoBytes::from(raw.as_bytes().to_vec()))
        });
    let end_key = query
        .get("end_b64")
        .and_then(|raw| decode_base64(raw))
        .map(ProtoBytes::from)
        .or_else(|| {
            query
                .get("end")
                .filter(|raw| !raw.is_empty())
                .map(|raw| ProtoBytes::from(raw.as_bytes().to_vec()))
        });
    response_payload(state.dispatch(
        Request::Scan {
            scope_id,
            start_key,
            end_key,
        },
        &headers,
    ))
}

pub(super) async fn tx_begin(State(state): State<HttpState>, headers: HeaderMap) -> HttpResponse {
    response_payload(state.dispatch(Request::TxBegin, &headers))
}

pub(super) async fn tx_commit(
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

pub(super) fn resolve_scope(state: &HttpState, name: &str) -> Result<u32, Box<HttpResponse>> {
    let registry = state.server.db().inner().scope_names.read();
    registry.get(name).copied().ok_or_else(|| {
        Box::new(json_error(
            StatusCode::NOT_FOUND,
            "scope_not_found",
            &format!("scope not found: {name}"),
        ))
    })
}

fn key_from(path_key: &str, query: &HashMap<String, String>) -> Option<ProtoBytes> {
    match query.get("key_b64") {
        Some(encoded) => decode_base64(encoded).map(ProtoBytes::from),
        None => Some(ProtoBytes::from(path_key.as_bytes().to_vec())),
    }
}

fn value_from(query: &HashMap<String, String>, body: &[u8]) -> Option<ProtoBytes> {
    if let Some(encoded) = query.get("value_b64") {
        return decode_base64(encoded).map(ProtoBytes::from);
    }
    if !body.is_empty() {
        if let Ok(parsed) = serde_json::from_slice::<Value>(body) {
            if let Some(encoded) = parsed.get("value_b64").and_then(Value::as_str) {
                return decode_base64(encoded).map(ProtoBytes::from);
            }
        }
    }
    Some(ProtoBytes::copy_from_slice(body))
}

fn field_bytes(parsed: &Value, field: &str) -> Option<ProtoBytes> {
    parsed
        .get(field)
        .and_then(Value::as_str)
        .and_then(decode_base64)
        .map(ProtoBytes::from)
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
