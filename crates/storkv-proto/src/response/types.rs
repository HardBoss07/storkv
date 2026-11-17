use bytes::Bytes;

use crate::error::StorKvWireError;

/// Visible-version metadata of a key, used by migration and TTL tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WireKeyMeta {
    /// Absolute Unix epoch milliseconds, `0` when the key is permanent.
    pub expires_at_ms: u64,
    /// Sequence number of the visible version.
    pub seq: u64,
}

/// Identifier and name of a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireScopeHandle {
    /// Opaque scope identifier used by data requests.
    pub scope_id: u32,
    /// Unique scope name.
    pub name: String,
}

/// Runtime counters returned by a telemetry query.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireTelemetrySnapshot {
    /// Total write operations.
    pub writes_total: u64,
    /// Total read operations.
    pub reads_total: u64,
    /// Total optimistic-concurrency write conflicts.
    pub write_conflicts_total: u64,
    /// Total compaction runs.
    pub compactions_total: u64,
    /// Total WAL sync operations.
    pub fsyncs_total: u64,
    /// Total successful backups.
    pub backups_completed_total: u64,
    /// Total failed backups.
    pub backups_failed_total: u64,
    /// Unix epoch milliseconds of the last successful backup.
    pub last_backup_timestamp_ms: u64,
    /// Sequence number captured by the last successful backup.
    pub last_backup_seq: u64,
    /// Total WAL segments sent to the backup sink.
    pub wal_segments_shipped_total: u64,
}

/// Result of a completed full backup generation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireBackupReport {
    /// Unique identifier of the new generation.
    pub generation_id: String,
    /// Sequence number captured by the backup.
    pub sequence: u64,
    /// Number of files in the generation.
    pub files: u32,
    /// Total bytes in the generation.
    pub bytes: u64,
    /// Number of old full generations removed by retention.
    pub generations_removed: u32,
    /// Number of old WAL segments removed by retention.
    pub wal_segments_removed: u32,
    /// Bytes freed by retention pruning.
    pub bytes_freed: u64,
}

/// Live status of the autonomous backup worker.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireBackupStatus {
    /// True when the worker is running.
    pub enabled: bool,
    /// Backup root path, when configured.
    pub backup_root: Option<String>,
    /// Unix epoch milliseconds of the last successful backup.
    pub last_backup_timestamp_ms: u64,
    /// Sequence number of the last successful backup.
    pub last_backup_seq: u64,
    /// Total successful backups.
    pub backups_completed_total: u64,
    /// Total failed backups.
    pub backups_failed_total: u64,
    /// Total WAL segments sent to the sink.
    pub wal_segments_shipped_total: u64,
    /// Milliseconds until the next scheduled full backup.
    pub next_full_backup_in_ms: u64,
    /// Detail of the most recent backup failure, if any.
    pub last_error: Option<String>,
    /// Sink description, such as `local` or `command`.
    pub sink: String,
}

/// One verification failure inside a backup generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireBackupVerifyError {
    /// Path of the file that failed verification.
    pub path: String,
    /// Failure detail.
    pub detail: String,
}

/// Result of a backup verification pass.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireBackupVerifyReport {
    /// Number of generations checked.
    pub generations_checked: u32,
    /// Number of files checked.
    pub files_checked: u64,
    /// Number of bytes read and verified.
    pub bytes_checked: u64,
    /// Number of WAL segments checked.
    pub wal_segments_checked: u64,
    /// Verification failures. An empty list means success.
    pub errors: Vec<WireBackupVerifyError>,
}

impl WireBackupVerifyReport {
    /// Return true when the report contains no errors.
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Type of an archived backup generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireBackupKind {
    /// A full database image.
    Full,
    /// A stream of sealed WAL segments.
    Wal,
}

/// One archived generation in the backup catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireBackupEntry {
    /// Unique identifier of the generation.
    pub generation_id: String,
    /// Compression codec used by the generation.
    pub compression: String,
    /// Unix epoch milliseconds when the generation was created.
    pub created_at_ms: u64,
    /// Oldest sequence number in the generation.
    pub min_seq: u64,
    /// Newest sequence number in the generation.
    pub max_seq: u64,
    /// Total bytes in the generation.
    pub total_bytes: u64,
    /// Number of files in the generation.
    pub files: u32,
    /// Generation kind.
    pub kind: WireBackupKind,
}

const _: () = assert!(std::mem::size_of::<WireBackupEntry>() <= 96);

/// Per-level SSTable statistics for the admin dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireLevelStat {
    /// Level number from L0 through L6.
    pub level: u8,
    /// Number of SSTable files in the level.
    pub file_count: u32,
    /// Total bytes of SSTable files in the level.
    pub size_bytes: u64,
}

/// Per-scope activity statistics for the admin dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireScopeStat {
    /// Identifier of the scope.
    pub scope_id: u32,
    /// Name of the scope.
    pub name: String,
    /// Total writes to the scope.
    pub writes_total: u64,
    /// Total reads from the scope.
    pub reads_total: u64,
    /// Number of SSTable files that hold scope data.
    pub sst_files: u32,
}

/// Per-transport connection counters. Appended to [`WireAdminStats`]; peers
/// that predate the extension simply report an empty list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireTransportStat {
    /// Transport kind code. See the server transport table.
    pub kind: u8,
    /// Total connections accepted since start.
    pub connections_total: u64,
    /// Connections that are currently open.
    pub active_connections: u64,
    /// Total requests handled on the transport.
    pub requests_total: u64,
}

/// Rich admin statistics: L0..L6 SSTable distribution, per-scope metrics, and
/// the observed WAL commit (fsync) latency percentile.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireAdminStats {
    /// Per-level SSTable distribution.
    pub levels: Vec<WireLevelStat>,
    /// Per-scope activity counters.
    pub scopes: Vec<WireScopeStat>,
    /// 99th percentile of the observed fsync latency, in microseconds.
    pub fsync_latency_p99_us: u64,
    /// Number of latency samples behind the percentile.
    pub fsync_latency_samples: u64,
    /// Per-transport counters.
    pub transports: Vec<WireTransportStat>,
    /// Total successful backups.
    pub backups_completed_total: u64,
    /// Total failed backups.
    pub backups_failed_total: u64,
    /// Unix epoch milliseconds of the last successful backup.
    pub last_backup_timestamp_ms: u64,
    /// Sequence number of the last successful backup.
    pub last_backup_seq: u64,
    /// Total WAL segments sent to the backup sink.
    pub wal_segments_shipped_total: u64,
}

/// Change event pushed to a watch subscriber.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireChangeEvent {
    /// A key received a new value.
    Put {
        /// Identifier of the scope.
        scope_id: u32,
        /// User key that changed.
        key: Bytes,
        /// New value bytes.
        value: Bytes,
        /// Commit sequence number of the change.
        seq: u64,
    },
    /// A key was deleted.
    Delete {
        /// Identifier of the scope.
        scope_id: u32,
        /// User key that was deleted.
        key: Bytes,
        /// Commit sequence number of the change.
        seq: u64,
    },
    /// A scope was dropped.
    DropScope {
        /// Identifier of the dropped scope.
        scope_id: u32,
        /// Commit sequence number of the change.
        seq: u64,
    },
}

/// Response message in the StorKV native protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Response {
    /// Reply to [`crate::Request::Ping`].
    Pong,
    /// Generic success with no payload.
    Empty,
    /// Reply to [`crate::Request::TxBegin`] with the read sequence number.
    TxStarted {
        /// Sequence number that the transaction must read and validate.
        read_seq: u64,
    },
    /// Reply to a read. `None` means the key does not exist.
    Value(Option<Bytes>),
    /// Reply to a range read as ordered key/value pairs.
    Scan(Vec<(Bytes, Bytes)>),
    /// Reply to a scope creation.
    Scope(WireScopeHandle),
    /// Reply to a telemetry query.
    Telemetry(WireTelemetrySnapshot),
    /// Reply to a health check.
    HealthOk,
    /// One watch event pushed on a subscribed connection.
    WatchEvent(WireChangeEvent),
    /// Reply to an admin statistics request.
    AdminStats(WireAdminStats),
    /// Reply to a backup status request.
    BackupStatus(WireBackupStatus),
    /// Reply to a backup trigger.
    BackupReport(WireBackupReport),
    /// Reply to a backup verification request.
    BackupVerifyReport(WireBackupVerifyReport),
    /// Reply to a backup catalog request.
    BackupCatalog(Vec<WireBackupEntry>),
    /// Reply to a key metadata request. `None` means the key does not exist.
    KeyMeta(Option<WireKeyMeta>),
    /// Operation failure.
    Error(StorKvWireError),
}
