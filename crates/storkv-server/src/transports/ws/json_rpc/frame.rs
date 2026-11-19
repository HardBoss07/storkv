//! JSON response framing for the WebSocket transport.

use serde_json::json;
use storkv_proto::Response;

use crate::transports::http::json::{code_for_wire_error, response_value};

pub(crate) fn success(id: u64, response: Response) -> String {
    match response {
        Response::Error(wire_error) => error(
            id,
            code_for_wire_error(&wire_error),
            &wire_error.to_string(),
        ),
        other => json!({"id": id, "ok": true, "result": response_value(other)}).to_string(),
    }
}

pub(crate) fn error(id: u64, code: &str, message: &str) -> String {
    json!({
        "id": id,
        "ok": false,
        "error": {"code": code, "message": message},
    })
    .to_string()
}

pub(crate) fn watching(id: u64) -> String {
    json!({"id": id, "ok": true, "watching": true}).to_string()
}
