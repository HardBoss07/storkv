//! JSON request parsing and response framing for the WebSocket transport.
//!
//! Browser tooling may send text frames such as
//! `{"id": 1, "op": "get", "scope": "users", "key_b64": "dXNlcjox"}`. The
//! binary `STORKPRT` frame path remains the primary client protocol; this
//! module only translates the documented JSON surface onto the same
//! [`Request`] values so both paths share dispatch and policy.

use serde_json::Value;
use storkv_proto::Request;

use crate::transports::http::HttpState;

mod frame;
mod operation;

pub(super) use frame::{error, success, watching};

pub(super) const MALFORMED_REQUEST: &str = "malformed JSON request";
pub(super) const UNKNOWN_OPERATION: &str = "unknown or malformed websocket operation";

pub(super) struct ParsedRequest {
    pub(super) id: u64,
    pub(super) request: Request,
}

pub(super) struct ParseError {
    pub(super) id: u64,
    pub(super) message: String,
}

pub(super) fn parse(state: &HttpState, text: &str) -> Result<ParsedRequest, ParseError> {
    let value: Value = serde_json::from_str(text).map_err(|_| ParseError {
        id: 0,
        message: MALFORMED_REQUEST.to_string(),
    })?;
    let id = value.get("id").and_then(Value::as_u64).unwrap_or(0);
    let request =
        operation::parse_operation(state, &value).map_err(|message| ParseError { id, message })?;
    Ok(ParsedRequest { id, request })
}
