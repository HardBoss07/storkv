//! REST health, telemetry, and admin handlers.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response as HttpResponse;
use serde_json::Value;
use storkv_proto::Request;

use super::HttpState;
use super::json::{json_error, response_payload};

pub(super) async fn health(State(state): State<HttpState>, headers: HeaderMap) -> HttpResponse {
    response_payload(state.dispatch(Request::HealthCheck, &headers))
}

pub(super) async fn ping(State(state): State<HttpState>, headers: HeaderMap) -> HttpResponse {
    response_payload(state.dispatch(Request::Ping, &headers))
}

pub(super) async fn telemetry(State(state): State<HttpState>, headers: HeaderMap) -> HttpResponse {
    response_payload(state.dispatch(Request::TelemetryQuery, &headers))
}

pub(super) async fn admin_stats(
    State(state): State<HttpState>,
    headers: HeaderMap,
) -> HttpResponse {
    response_payload(state.dispatch(Request::AdminStats, &headers))
}

pub(super) async fn checkpoint(
    State(state): State<HttpState>,
    headers: HeaderMap,
    body: Bytes,
) -> HttpResponse {
    let parsed = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    let Some(dest_dir) = parsed.get("dest_dir").and_then(Value::as_str) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_body", "missing dest_dir");
    };
    response_payload(state.dispatch(
        Request::Checkpoint {
            dest_dir: dest_dir.to_owned(),
        },
        &headers,
    ))
}

pub(super) async fn compact(State(state): State<HttpState>, headers: HeaderMap) -> HttpResponse {
    response_payload(state.dispatch(Request::Compact, &headers))
}

pub(super) async fn trigger_backup(
    State(state): State<HttpState>,
    headers: HeaderMap,
    body: Bytes,
) -> HttpResponse {
    let parsed = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    let wait = parsed.get("wait").and_then(Value::as_bool).unwrap_or(false);
    response_payload(state.dispatch(Request::TriggerBackup { wait }, &headers))
}

pub(super) async fn create_snapshot(
    State(state): State<HttpState>,
    headers: HeaderMap,
    body: Bytes,
) -> HttpResponse {
    let parsed = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    let Some(name) = parsed.get("name").and_then(Value::as_str) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_body", "missing name");
    };
    response_payload(state.dispatch(
        Request::CreateSnapshot {
            name: name.to_owned(),
        },
        &headers,
    ))
}

pub(super) async fn unpin_snapshot(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> HttpResponse {
    response_payload(state.dispatch(Request::UnpinSnapshot { name }, &headers))
}

pub(super) async fn verify_backup(
    State(state): State<HttpState>,
    headers: HeaderMap,
    body: Bytes,
) -> HttpResponse {
    let parsed = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    let target_seq = parsed.get("target_seq").and_then(Value::as_u64);
    response_payload(state.dispatch(Request::VerifyBackup { target_seq }, &headers))
}

pub(super) async fn list_backups(
    State(state): State<HttpState>,
    headers: HeaderMap,
) -> HttpResponse {
    response_payload(state.dispatch(Request::ListBackups, &headers))
}
