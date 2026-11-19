//! REST handlers for individual keys.

use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response as HttpResponse};
use bytes::Bytes as ProtoBytes;
use serde_json::Value;
use storkv_proto::{Request, Response, WireMergeOp};

use super::super::HttpState;
use super::super::json::{decode_base64, json_error, response_payload};
use super::support::{ScopeQuery, key_from, resolve_scope, value_from};

pub(crate) async fn get_key(
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

pub(crate) async fn put_key(
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

pub(crate) async fn delete_key(
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

pub(crate) async fn merge(
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
