//! REST handlers for range and prefix scans.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response as HttpResponse;
use bytes::Bytes as ProtoBytes;
use serde_json::Value;
use storkv_proto::Request;

use super::super::HttpState;
use super::super::json::{decode_base64, json_error, response_payload};
use super::support::{ScopeQuery, field_bytes, resolve_scope};

pub(crate) async fn delete_range(
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

pub(crate) async fn scan(
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
                limit: None,
                reverse: false,
                keys_only: false,
            },
            &headers,
        ));
    }
    if let Some(prefix) = query.get("prefix").filter(|prefix| !prefix.is_empty()) {
        return response_payload(state.dispatch(
            Request::ScanPrefix {
                scope_id,
                prefix: ProtoBytes::from(prefix.as_bytes().to_vec()),
                limit: None,
                reverse: false,
                keys_only: false,
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
            limit: None,
            reverse: false,
            keys_only: false,
        },
        &headers,
    ))
}
