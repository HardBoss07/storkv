use storkv_core::StorkvError;
use storkv_proto::{Response, StorkvWireError};

pub(super) fn result_to_empty(result: storkv_core::Result<()>) -> Response {
    match result {
        Ok(()) => Response::Empty,
        Err(error) => Response::Error(core_error_to_wire(error)),
    }
}

/// Convert a core engine error into the wire error type.
pub fn core_error_to_wire(error: StorkvError) -> StorkvWireError {
    match error {
        StorkvError::DatabaseLocked(path) => {
            StorkvWireError::DatabaseLocked(path.display().to_string())
        }
        StorkvError::Corruption(message) => StorkvWireError::Corruption(message),
        StorkvError::WriteConflict { scope_id, key } => {
            StorkvWireError::WriteConflict { scope_id, key }
        }
        StorkvError::ScopeNotFound(name) => StorkvWireError::ScopeNotFound(name),
        StorkvError::ScopeIdNotFound(scope_id) => StorkvWireError::ScopeIdNotFound(scope_id),
        StorkvError::ScopeAlreadyExists(name) => StorkvWireError::ScopeAlreadyExists(name),
        StorkvError::ScopeDropped(scope_id) => StorkvWireError::ScopeDropped(scope_id),
        StorkvError::SnapshotNotFound(name) => StorkvWireError::SnapshotNotFound(name),
        StorkvError::WatchCursorExpired {
            requested,
            oldest_available,
        } => StorkvWireError::WatchCursorExpired {
            requested,
            oldest_available,
        },
        StorkvError::NotPrimary => StorkvWireError::NotPrimary,
        StorkvError::StaleReplica { lag_ms, max_lag_ms } => {
            StorkvWireError::StaleReplica { lag_ms, max_lag_ms }
        }
        StorkvError::TooManySubscribers { count, max } => {
            StorkvWireError::TooManySubscribers { count, max }
        }
        StorkvError::ReplayBudgetExceeded { events, bytes } => {
            StorkvWireError::ReplayBudgetExceeded { events, bytes }
        }
        StorkvError::KeyTooLarge { max, actual } => StorkvWireError::KeyTooLarge { max, actual },
        StorkvError::ValueTooLarge { max, actual } => {
            StorkvWireError::ValueTooLarge { max, actual }
        }
        StorkvError::InvalidOperation(message) => StorkvWireError::InvalidOperation(message),
        StorkvError::Io(error) => StorkvWireError::Io(error.to_string()),
        other => StorkvWireError::InvalidOperation(other.to_string()),
    }
}
