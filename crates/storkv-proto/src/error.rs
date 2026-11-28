//! Wire error codes shared by clients and servers.
//!
//! Each variant maps to one stable tag byte. Peers must not reorder or reuse
//! the tags (`FORMAT.md`). The payload holds the tag and the variant fields.

mod codec;

use thiserror::Error;

pub(crate) use codec::{decode_error, encode_error};

/// Wire-level error codes shared by clients and servers.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorkvWireError {
    /// Another process holds the database directory lock.
    #[error("Database locked: {0}")]
    DatabaseLocked(String),
    /// A checksum or structure check failed on stored data.
    #[error("Storage corruption: {0}")]
    Corruption(String),
    /// Optimistic concurrency validation found a changed read key.
    #[error("Write conflict in scope {scope_id}")]
    WriteConflict {
        /// Identifier of the scope that reported the conflict.
        scope_id: u32,
        /// User key that changed after the read.
        key: Vec<u8>,
    },
    /// The named scope does not exist.
    #[error("Scope not found: {0}")]
    ScopeNotFound(String),
    /// The scope identifier does not exist.
    #[error("Scope ID not found: {0}")]
    ScopeIdNotFound(u32),
    /// A scope with the requested name already exists.
    #[error("Scope already exists: {0}")]
    ScopeAlreadyExists(String),
    /// The scope was dropped after the request started.
    #[error("Scope dropped: {0}")]
    ScopeDropped(u32),
    /// The named snapshot does not exist.
    #[error("Snapshot not found: {0}")]
    SnapshotNotFound(String),
    /// A key is longer than the protocol limit.
    #[error("Key exceeds maximum length: {actual} > {max}")]
    KeyTooLarge {
        /// Maximum permitted key length in bytes.
        max: usize,
        /// Actual key length in bytes.
        actual: usize,
    },
    /// A value is longer than the protocol limit.
    #[error("Value exceeds maximum length: {actual} > {max}")]
    ValueTooLarge {
        /// Maximum permitted value length in bytes.
        max: usize,
        /// Actual value length in bytes.
        actual: usize,
    },
    /// The server rejected a semantically invalid request.
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
    /// An input or output operation failed.
    #[error("IO error: {0}")]
    Io(String),
    /// A frame was malformed or used an unsupported version.
    #[error("Protocol error: {0}")]
    Protocol(String),
    /// The requested watch cursor is older than the retained WAL history.
    #[error("Watch cursor {requested} expired (oldest available: {oldest_available})")]
    WatchCursorExpired {
        /// Cursor sequence requested by the subscriber.
        requested: u64,
        /// Oldest sequence still available for replay.
        oldest_available: u64,
    },
    /// Strict admin mode rejected an administrative request.
    #[error("Unauthorized: this request requires an admin connection")]
    Unauthorized,
    /// This node is a replica and cannot accept writes.
    #[error("This node is a replica and cannot accept writes")]
    NotPrimary,
    /// The replica lags behind the primary by more than the allowed maximum.
    #[error("Replica lag {lag_ms} ms exceeds the maximum of {max_lag_ms} ms")]
    StaleReplica {
        /// Observed replication lag in milliseconds.
        lag_ms: u64,
        /// Maximum permitted lag in milliseconds.
        max_lag_ms: u64,
    },
    /// A stream registry reached its subscriber limit.
    #[error("Too many subscribers: {count} active (maximum: {max})")]
    TooManySubscribers {
        /// Number of active subscribers.
        count: usize,
        /// Maximum number of subscribers.
        max: usize,
    },
    /// A stream replay would exceed the event or byte budget.
    #[error("Replay budget exceeded: {events} events and {bytes} bytes")]
    ReplayBudgetExceeded {
        /// Number of events in the replay request.
        events: usize,
        /// Total payload bytes in the replay request.
        bytes: usize,
    },
}
