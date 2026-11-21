//! JSON helpers that map [`storkv_proto::Response`] values to HTTP responses.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response as HttpResponse};
use base64::Engine;
use serde_json::{Value, json};
use storkv_proto::{
    Response, StorkvWireError, WireBackupEntry, WireBackupKind, WireBackupReport, WireBackupStatus,
    WireBackupVerifyReport, WireChangeEvent,
};

pub(crate) fn encode_base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub(crate) fn decode_base64(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text))
        .ok()
}

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

pub(crate) fn response_payload(response: Response) -> HttpResponse {
    match response {
        Response::Error(error) => error_from_wire(&error),
        Response::HealthOk => Json(json!({"status": "ok"})).into_response(),
        Response::Pong => Json(json!({"pong": true})).into_response(),
        other => Json(response_value(other)).into_response(),
    }
}

pub(crate) fn response_value(response: Response) -> Value {
    match response {
        Response::Empty => json!({"ok": true}),
        Response::Pong => json!({"pong": true}),
        Response::HealthOk => json!({"status": "ok"}),
        Response::TxStarted { read_seq } => json!({"read_seq": read_seq}),
        Response::Value(value) => match value {
            Some(bytes) => json!({"found": true, "value_b64": encode_base64(&bytes)}),
            None => json!({"found": false}),
        },
        Response::Scan(pairs) => json!({
            "pairs": pairs
                .into_iter()
                .map(|(key, value)| json!({
                    "key_b64": encode_base64(&key),
                    "value_b64": encode_base64(&value),
                }))
                .collect::<Vec<_>>()
        }),
        Response::Scope(handle) => json!({"scope_id": handle.scope_id, "name": handle.name}),
        Response::Telemetry(snapshot) => json!({
            "writes_total": snapshot.writes_total,
            "reads_total": snapshot.reads_total,
            "write_conflicts_total": snapshot.write_conflicts_total,
            "compactions_total": snapshot.compactions_total,
            "fsyncs_total": snapshot.fsyncs_total,
            "backups_completed_total": snapshot.backups_completed_total,
            "backups_failed_total": snapshot.backups_failed_total,
            "last_backup_timestamp_ms": snapshot.last_backup_timestamp_ms,
            "last_backup_seq": snapshot.last_backup_seq,
            "wal_segments_shipped_total": snapshot.wal_segments_shipped_total,
        }),
        Response::AdminStats(stats) => json!({
            "levels": stats.levels.iter().map(|level| json!({
                "level": level.level,
                "file_count": level.file_count,
                "size_bytes": level.size_bytes,
            })).collect::<Vec<_>>(),
            "scopes": stats.scopes.iter().map(|scope| json!({
                "scope_id": scope.scope_id,
                "name": scope.name,
                "writes_total": scope.writes_total,
                "reads_total": scope.reads_total,
                "sst_files": scope.sst_files,
            })).collect::<Vec<_>>(),
            "fsync_latency_p99_us": stats.fsync_latency_p99_us,
            "fsync_latency_samples": stats.fsync_latency_samples,
            "transports": stats.transports.iter().map(|transport| json!({
                "kind": transport.kind,
                "connections_total": transport.connections_total,
                "active_connections": transport.active_connections,
                "requests_total": transport.requests_total,
            })).collect::<Vec<_>>(),
            "backups_completed_total": stats.backups_completed_total,
            "backups_failed_total": stats.backups_failed_total,
            "last_backup_timestamp_ms": stats.last_backup_timestamp_ms,
            "last_backup_seq": stats.last_backup_seq,
            "wal_segments_shipped_total": stats.wal_segments_shipped_total,
        }),
        Response::BackupStatus(status) => backup_status_value(status),
        Response::BackupReport(report) => backup_report_value(report),
        Response::BackupVerifyReport(report) => verify_report_value(report),
        Response::BackupCatalog(entries) => json!({
            "generations": entries.iter().map(catalog_entry_value).collect::<Vec<_>>()
        }),
        Response::WatchEvent(event) => json!({"event": watch_event_value(event)}),
        Response::Error(error) => json!({
            "error": {"code": code_for_wire_error(&error), "message": error.to_string()}
        }),
        _ => json!({"ok": true}),
    }
}

fn backup_status_value(status: WireBackupStatus) -> Value {
    json!({
        "enabled": status.enabled,
        "backup_root": status.backup_root,
        "last_backup_timestamp_ms": status.last_backup_timestamp_ms,
        "last_backup_seq": status.last_backup_seq,
        "backups_completed_total": status.backups_completed_total,
        "backups_failed_total": status.backups_failed_total,
        "wal_segments_shipped_total": status.wal_segments_shipped_total,
        "next_full_backup_in_ms": status.next_full_backup_in_ms,
        "last_error": status.last_error,
        "sink": status.sink,
    })
}

fn backup_report_value(report: WireBackupReport) -> Value {
    json!({
        "generation_id": report.generation_id,
        "sequence": report.sequence,
        "files": report.files,
        "bytes": report.bytes,
        "generations_removed": report.generations_removed,
        "wal_segments_removed": report.wal_segments_removed,
        "bytes_freed": report.bytes_freed,
    })
}

fn verify_report_value(report: WireBackupVerifyReport) -> Value {
    json!({
        "generations_checked": report.generations_checked,
        "files_checked": report.files_checked,
        "bytes_checked": report.bytes_checked,
        "wal_segments_checked": report.wal_segments_checked,
        "errors": report.errors.iter().map(|error| json!({
            "path": error.path,
            "detail": error.detail,
        })).collect::<Vec<_>>(),
    })
}

fn catalog_entry_value(entry: &WireBackupEntry) -> Value {
    json!({
        "generation_id": entry.generation_id,
        "kind": match entry.kind {
            WireBackupKind::Full => "full",
            WireBackupKind::Wal => "wal",
            _ => "unknown",
        },
        "created_at_ms": entry.created_at_ms,
        "min_seq": entry.min_seq,
        "max_seq": entry.max_seq,
        "total_bytes": entry.total_bytes,
        "files": entry.files,
        "compression": entry.compression,
    })
}

pub(crate) fn watch_event_value(event: WireChangeEvent) -> Value {
    match event {
        WireChangeEvent::Put {
            scope_id,
            key,
            value,
            seq,
            expires_at_ms,
        } => json!({
            "kind": "put",
            "scope_id": scope_id,
            "key_b64": encode_base64(&key),
            "value_b64": encode_base64(&value),
            "seq": seq,
            "expires_at_ms": expires_at_ms,
        }),
        WireChangeEvent::Delete { scope_id, key, seq } => json!({
            "kind": "delete",
            "scope_id": scope_id,
            "key_b64": encode_base64(&key),
            "seq": seq,
        }),
        WireChangeEvent::DropScope { scope_id, seq } => json!({
            "kind": "drop_scope",
            "scope_id": scope_id,
            "seq": seq,
        }),
        WireChangeEvent::InvalidateScope { scope_id, seq } => json!({
            "kind": "invalidate_scope",
            "scope_id": scope_id,
            "seq": seq,
        }),
        _ => json!(null),
    }
}
