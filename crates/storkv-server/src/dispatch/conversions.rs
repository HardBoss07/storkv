use storkv_core::StorKvError;
use storkv_proto::{Response, StorKvWireError};

pub(super) fn result_to_empty(result: storkv_core::Result<()>) -> Response {
    match result {
        Ok(()) => Response::Empty,
        Err(error) => Response::Error(core_error_to_wire(error)),
    }
}

/// Convert a core engine error into the wire error type.
pub fn core_error_to_wire(error: StorKvError) -> StorKvWireError {
    match error {
        StorKvError::DatabaseLocked(path) => {
            StorKvWireError::DatabaseLocked(path.display().to_string())
        }
        StorKvError::Corruption(message) => StorKvWireError::Corruption(message),
        StorKvError::WriteConflict { scope_id, key } => {
            StorKvWireError::WriteConflict { scope_id, key }
        }
        StorKvError::ScopeNotFound(name) => StorKvWireError::ScopeNotFound(name),
        StorKvError::ScopeIdNotFound(scope_id) => StorKvWireError::ScopeIdNotFound(scope_id),
        StorKvError::ScopeAlreadyExists(name) => StorKvWireError::ScopeAlreadyExists(name),
        StorKvError::ScopeDropped(scope_id) => StorKvWireError::ScopeDropped(scope_id),
        StorKvError::SnapshotNotFound(name) => StorKvWireError::SnapshotNotFound(name),
        StorKvError::KeyTooLarge { max, actual } => StorKvWireError::KeyTooLarge { max, actual },
        StorKvError::ValueTooLarge { max, actual } => {
            StorKvWireError::ValueTooLarge { max, actual }
        }
        StorKvError::InvalidOperation(message) => StorKvWireError::InvalidOperation(message),
        StorKvError::Io(error) => StorKvWireError::Io(error.to_string()),
        other => StorKvWireError::InvalidOperation(other.to_string()),
    }
}
