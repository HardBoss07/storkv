//! JSON field extraction helpers for WebSocket operations.

use bytes::Bytes;
use serde_json::Value;

use crate::transports::http::HttpState;
use crate::transports::http::json::decode_base64;

use super::super::UNKNOWN_OPERATION;

pub(super) fn scope_id(state: &HttpState, value: &Value) -> Result<u32, String> {
    if let Some(scope_id) = value.get("scope_id").and_then(Value::as_u64) {
        return u32::try_from(scope_id).map_err(|_| UNKNOWN_OPERATION.to_string());
    }
    let name = value
        .get("scope")
        .and_then(Value::as_str)
        .ok_or_else(|| UNKNOWN_OPERATION.to_string())?;
    state
        .server
        .db()
        .inner()
        .scope_names
        .read()
        .get(name)
        .copied()
        .ok_or_else(|| format!("scope not found: {name}"))
}

pub(super) fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| UNKNOWN_OPERATION.to_string())
}

pub(super) fn required_key_field(
    value: &Value,
    field: &str,
    field_b64: &str,
) -> Result<Bytes, String> {
    key_field(value, field, field_b64)
}

pub(super) fn key_field(value: &Value, field: &str, field_b64: &str) -> Result<Bytes, String> {
    optional_key_field(value, field, Some(field_b64)).ok_or_else(|| UNKNOWN_OPERATION.to_string())
}

pub(super) fn optional_key_field(
    value: &Value,
    field: &str,
    field_b64: Option<&str>,
) -> Option<Bytes> {
    if let Some(encoded) = field_b64
        .and_then(|name| value.get(name))
        .and_then(Value::as_str)
    {
        if let Some(decoded) = decode_base64(encoded) {
            return Some(Bytes::from(decoded));
        }
    }
    value
        .get(field)
        .and_then(Value::as_str)
        .map(|text| Bytes::copy_from_slice(text.as_bytes()))
}
