use std::path::PathBuf;
use thiserror::Error;

/// Result alias for StorKV operations.
pub type Result<T> = std::result::Result<T, StorKvError>;

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum StorKvError {
    #[error("Database is locked by another process: {0}")]
    DatabaseLocked(PathBuf),

    #[error("Storage corruption detected: {0}")]
    Corruption(String),

    #[error(
        "Write conflict detected during OCC commit validation: key `{key:?}` in scope `{scope_id}`"
    )]
    WriteConflict { scope_id: u32, key: Vec<u8> },

    #[error("Scope not found: {0}")]
    ScopeNotFound(String),

    #[error("Scope id not found: {0}")]
    ScopeIdNotFound(u32),

    #[error("Scope already exists: {0}")]
    ScopeAlreadyExists(String),

    #[error("Scope has been dropped: {0}")]
    ScopeDropped(u32),

    #[error("Snapshot not found: {0}")]
    SnapshotNotFound(String),

    #[error("Key exceeds maximum length of {max} bytes (actual: {actual})")]
    KeyTooLarge { max: usize, actual: usize },

    #[error("Value exceeds maximum length of {max} bytes (actual: {actual})")]
    ValueTooLarge { max: usize, actual: usize },

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("Underlying I/O error: {0}")]
    Io(#[from] std::io::Error),
}
