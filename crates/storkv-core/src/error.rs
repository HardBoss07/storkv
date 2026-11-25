use std::path::PathBuf;
use thiserror::Error;

/// Result alias for StorKV operations.
pub type Result<T> = std::result::Result<T, StorkvError>;

/// Error type for all StorKV operations.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum StorkvError {
    /// Another process holds the database directory lock.
    #[error("Database is locked by another process: {0}")]
    DatabaseLocked(PathBuf),

    /// A checksum or structure check failed on stored data.
    #[error("Storage corruption detected: {0}")]
    Corruption(String),

    /// Optimistic concurrency validation found a changed read key.
    #[error(
        "Write conflict detected during OCC commit validation: key `{key:?}` in scope `{scope_id}`"
    )]
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
    #[error("Scope id not found: {0}")]
    ScopeIdNotFound(u32),

    /// A scope with the requested name already exists.
    #[error("Scope already exists: {0}")]
    ScopeAlreadyExists(String),

    /// The scope was dropped after the operation started.
    #[error("Scope has been dropped: {0}")]
    ScopeDropped(u32),

    /// The named snapshot does not exist.
    #[error("Snapshot not found: {0}")]
    SnapshotNotFound(String),

    /// A key is longer than [`crate::format::MAX_KEY_LEN`].
    #[error("Key exceeds maximum length of {max} bytes (actual: {actual})")]
    KeyTooLarge {
        /// Maximum permitted key length in bytes.
        max: usize,
        /// Actual key length in bytes.
        actual: usize,
    },

    /// A value is longer than [`crate::format::MAX_VAL_LEN`].
    #[error("Value exceeds maximum length of {max} bytes (actual: {actual})")]
    ValueTooLarge {
        /// Maximum permitted value length in bytes.
        max: usize,
        /// Actual value length in bytes.
        actual: usize,
    },

    /// The operation is not valid in the current state.
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    /// A foreground write stopped at an L0 or WAL backpressure watermark.
    #[error("Write stalled: {0}")]
    WriteStalled(String),

    /// A write time to live violates the scope policy.
    #[error("Invalid TTL: {0}")]
    InvalidTtl(String),

    /// A backup sink operation failed.
    #[error("Backup sink `{sink}` failed: {detail}")]
    BackupSink {
        /// Sink name.
        sink: String,
        /// Failure detail.
        detail: String,
    },

    /// The requested backup generation does not exist.
    #[error("Backup generation not found: {0}")]
    BackupGenerationNotFound(String),

    /// A backup file failed verification.
    #[error("Backup corruption at `{path}`: {detail}")]
    BackupCorruption {
        /// File path that failed verification.
        path: String,
        /// Failure detail.
        detail: String,
    },

    /// The requested sequence is not inside the archived range.
    #[error("Requested sequence {requested} is outside the archived range [{oldest}, {newest}]")]
    TargetSequenceUnavailable {
        /// Sequence number that the caller requested.
        requested: u64,
        /// Oldest available sequence number.
        oldest: u64,
        /// Newest available sequence number.
        newest: u64,
    },

    /// A continuous backup worker is already running.
    #[error("Automatic backup is already running")]
    AutoBackupAlreadyRunning,

    /// No continuous backup worker is running.
    #[error("Automatic backup is not running")]
    AutoBackupNotRunning,

    /// The restore destination directory holds files.
    #[error("Restore target directory is not empty: {0}")]
    RestoreTargetNotEmpty(PathBuf),

    /// The requested watch cursor is older than the retained WAL history.
    #[error("Watch cursor {requested} expired (oldest available: {oldest_available})")]
    WatchCursorExpired {
        /// Cursor sequence requested by the subscriber.
        requested: u64,
        /// Oldest sequence still available for replay.
        oldest_available: u64,
    },

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

    /// A serialization or deserialization operation failed.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// An input or output operation failed.
    #[error("Underlying I/O error: {0}")]
    Io(#[from] std::io::Error),
}
