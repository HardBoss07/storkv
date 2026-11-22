//! Wire type definitions for protocol requests.

use bytes::Bytes;
use std::time::Duration;

use super::WireScopeOptions;

/// Merge operand sent over the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireMergeOp {
    /// Add an integer delta to the current counter value.
    CounterAdd(i64),
    /// Send a caller-defined operand to the merge operator.
    Custom(Bytes),
}

/// Pending transaction mutation sent in a commit request.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireTxMutation {
    /// Store a value. `0` in `expires_at_ms` means no expiry.
    Put {
        /// Value bytes to store.
        val: Bytes,
        /// Absolute Unix epoch milliseconds, or `0` for no expiry.
        expires_at_ms: u64,
    },
    /// Delete one key.
    Delete,
    /// Apply a merge operand to one key.
    Merge {
        /// Operand bytes for the merge operator.
        operand: Bytes,
    },
    /// Delete the range from the mutation key up to, but not including, `end_key`.
    DeleteRange {
        /// Exclusive end key of the range.
        end_key: Bytes,
    },
}

/// Request message in the StorKV native protocol.
///
/// Each variant maps to one opcode on the wire. The opcode table is in
/// `docs/crates/storkv-proto/wire_abi.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Request {
    /// Check that the server is reachable.
    Ping,
    /// Read one key from a scope.
    Get {
        /// Identifier of the target scope.
        scope_id: u32,
        /// User key to read.
        key: Bytes,
    },
    /// Store one key/value pair.
    Put {
        /// Identifier of the target scope.
        scope_id: u32,
        /// User key to store.
        key: Bytes,
        /// Value bytes to store.
        val: Bytes,
    },
    /// Store one key/value pair with a relative time to live.
    PutWithTtl {
        /// Identifier of the target scope.
        scope_id: u32,
        /// User key to store.
        key: Bytes,
        /// Value bytes to store.
        val: Bytes,
        /// Lifetime from now until the key expires.
        ttl: Duration,
    },
    /// Conditionally store one key/value pair in one round trip.
    PutIf {
        /// Identifier of the target scope.
        scope_id: u32,
        /// User key to write.
        key: Bytes,
        /// Condition that the newest visible state must satisfy.
        condition: crate::WirePutCondition,
        /// Value bytes to store when the condition matches.
        val: Bytes,
        /// Lifetime from now until the key expires. `None` is permanent.
        ttl: Option<Duration>,
    },
    /// Delete one key.
    Delete {
        /// Identifier of the target scope.
        scope_id: u32,
        /// User key to delete.
        key: Bytes,
    },
    /// Delete every key in a half-open range.
    DeleteRange {
        /// Identifier of the target scope.
        scope_id: u32,
        /// Inclusive start key of the range.
        start_key: Bytes,
        /// Exclusive end key of the range.
        end_key: Bytes,
    },
    /// Apply a merge operand to one key.
    Merge {
        /// Identifier of the target scope.
        scope_id: u32,
        /// User key to merge.
        key: Bytes,
        /// Merge operand.
        op: WireMergeOp,
    },
    /// Read an ordered range. `None` bounds are open.
    Scan {
        /// Identifier of the target scope.
        scope_id: u32,
        /// Inclusive start key, or `None` for the first key.
        start_key: Option<Bytes>,
        /// Exclusive end key, or `None` for the last key.
        end_key: Option<Bytes>,
    },
    /// Read every key that starts with a prefix.
    ScanPrefix {
        /// Identifier of the target scope.
        scope_id: u32,
        /// Prefix to match.
        prefix: Bytes,
    },
    /// Begin a transaction and receive a read sequence number.
    TxBegin,
    /// Commit a transaction after optimistic read-set validation.
    TxCommit {
        /// Sequence number captured by `TxBegin`.
        read_seq: u64,
        /// Keys that the transaction read.
        read_set: Vec<(u32, Bytes)>,
        /// Pending mutations keyed by scope identifier and user key.
        write_set: Vec<((u32, Bytes), WireTxMutation)>,
    },
    /// Create a scope.
    CreateScope {
        /// Unique scope name.
        name: String,
        /// Scope configuration.
        options: WireScopeOptions,
    },
    /// Drop a scope and delete its data.
    DropScope {
        /// Name of the scope to drop.
        name: String,
    },
    /// Pin a snapshot at the current sequence number.
    CreateSnapshot {
        /// Unique snapshot name.
        name: String,
    },
    /// Release a pinned snapshot.
    UnpinSnapshot {
        /// Name of the snapshot to release.
        name: String,
    },
    /// Copy a consistent database image to a directory.
    Checkpoint {
        /// Destination directory path.
        dest_dir: String,
    },
    /// Run compaction immediately.
    Compact,
    /// Subscribe the connection to change events.
    WatchSubscribe,
    /// Resume change events after a sequence cursor.
    WatchFrom {
        /// Last sequence number that the subscriber has processed.
        from_seq: u64,
    },
    /// Read runtime counters.
    TelemetryQuery,
    /// Check database health.
    HealthCheck,
    /// Read rich administrative statistics.
    AdminStats,
    /// Run a full backup now. `wait == false` only wakes the background worker.
    TriggerBackup {
        /// Block until the generation is published.
        wait: bool,
    },
    /// Read the live backup worker status.
    BackupStatus,
    /// Verify every archived generation, optionally as of `target_seq`.
    VerifyBackup {
        /// Sequence number to verify, or `None` for the newest state.
        target_seq: Option<u64>,
    },
    /// List archived backup generations.
    ListBackups,
    /// Fetch the visible version metadata of a key (exact TTL and sequence).
    KeyMeta {
        /// Identifier of the target scope.
        scope_id: u32,
        /// User key to inspect.
        key: Bytes,
    },
    /// Start a replication frame stream after a sequence cursor.
    ReplicationStart {
        /// Last sequence number that the replica has applied.
        from_seq: u64,
    },
    /// Select the consistency level for later data operations.
    ConfigureConsistency {
        /// Consistency level to apply.
        consistency: crate::Consistency,
    },
}
