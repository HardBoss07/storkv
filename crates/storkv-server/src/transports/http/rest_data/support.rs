//! Shared request parsing for the REST data plane.

use std::collections::HashMap;

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::Response as HttpResponse;
use bytes::Bytes as ProtoBytes;
use serde_json::Value;

use super::super::HttpState;
use super::super::json::{decode_base64, json_error};

pub(super) type ScopeQuery = Query<HashMap<String, String>>;

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

pub(super) fn key_from(path_key: &str, query: &HashMap<String, String>) -> Option<ProtoBytes> {
    match query.get("key_b64") {
        Some(encoded) => decode_base64(encoded).map(ProtoBytes::from),
        None => Some(ProtoBytes::from(path_key.as_bytes().to_vec())),
    }
}

pub(super) fn value_from(query: &HashMap<String, String>, body: &[u8]) -> Option<ProtoBytes> {
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

pub(super) fn field_bytes(parsed: &Value, field: &str) -> Option<ProtoBytes> {
    parsed
        .get(field)
        .and_then(Value::as_str)
        .and_then(decode_base64)
        .map(ProtoBytes::from)
}
