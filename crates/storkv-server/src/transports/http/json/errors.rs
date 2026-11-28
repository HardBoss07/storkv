//! Wire error to HTTP status and JSON body mapping.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response as HttpResponse};
use serde_json::json;
use storkv_proto::StorkvWireError;

pub(crate) fn json_error(status: StatusCode, code: &str, message: &str) -> HttpResponse {
    (
        status,
        Json(json!({"error": {"code": code, "message": message}})),
    )
        .into_response()
}

pub(crate) fn error_from_wire(error: &StorkvWireError) -> HttpResponse {
    let status = status_for_wire_error(error);
    let code = code_for_wire_error(error);
    json_error(status, code, &error.to_string())
}

fn status_for_wire_error(error: &StorkvWireError) -> StatusCode {
    match error {
        StorkvWireError::ScopeNotFound(_)
        | StorkvWireError::ScopeIdNotFound(_)
        | StorkvWireError::SnapshotNotFound(_) => StatusCode::NOT_FOUND,
        StorkvWireError::ScopeAlreadyExists(_) | StorkvWireError::WriteConflict { .. } => {
            StatusCode::CONFLICT
        }
        StorkvWireError::Unauthorized => StatusCode::FORBIDDEN,
        StorkvWireError::WatchCursorExpired { .. } => StatusCode::GONE,
        StorkvWireError::NotPrimary | StorkvWireError::StaleReplica { .. } => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        StorkvWireError::DatabaseLocked(_) => StatusCode::SERVICE_UNAVAILABLE,
        StorkvWireError::TooManySubscribers { .. } => StatusCode::TOO_MANY_REQUESTS,
        StorkvWireError::Corruption(_) | StorkvWireError::Io(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        _ => StatusCode::BAD_REQUEST,
    }
}

pub(crate) fn code_for_wire_error(error: &StorkvWireError) -> &'static str {
    match error {
        StorkvWireError::DatabaseLocked(_) => "database_locked",
        StorkvWireError::Corruption(_) => "corruption",
        StorkvWireError::WriteConflict { .. } => "write_conflict",
        StorkvWireError::ScopeNotFound(_) | StorkvWireError::ScopeIdNotFound(_) => {
            "scope_not_found"
        }
        StorkvWireError::ScopeAlreadyExists(_) => "scope_already_exists",
        StorkvWireError::ScopeDropped(_) => "scope_dropped",
        StorkvWireError::SnapshotNotFound(_) => "snapshot_not_found",
        StorkvWireError::KeyTooLarge { .. } => "key_too_large",
        StorkvWireError::ValueTooLarge { .. } => "value_too_large",
        StorkvWireError::InvalidOperation(_) => "invalid_operation",
        StorkvWireError::Io(_) => "io",
        StorkvWireError::Protocol(_) => "protocol",
        StorkvWireError::Unauthorized => "unauthorized",
        StorkvWireError::WatchCursorExpired { .. } => "watch_cursor_expired",
        StorkvWireError::NotPrimary => "not_primary",
        StorkvWireError::StaleReplica { .. } => "stale_replica",
        StorkvWireError::TooManySubscribers { .. } => "too_many_subscribers",
        StorkvWireError::ReplayBudgetExceeded { .. } => "replay_budget_exceeded",
        _ => "error",
    }
}
