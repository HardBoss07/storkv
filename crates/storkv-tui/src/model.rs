//! Polled telemetry, admin, and backup models for the dashboard.

use storkv_proto::{WireAdminStats, WireBackupStatus, WireTelemetrySnapshot};

/// A snapshot of telemetry data polled from the server.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TelemetryPoll {
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
    /// Sequence number of the last successful backup.
    pub last_backup_seq: u64,
    /// Total WAL segments sent to the backup sink.
    pub wal_segments_shipped_total: u64,
}

impl From<WireTelemetrySnapshot> for TelemetryPoll {
    fn from(snapshot: WireTelemetrySnapshot) -> Self {
        Self {
            writes_total: snapshot.writes_total,
            reads_total: snapshot.reads_total,
            write_conflicts_total: snapshot.write_conflicts_total,
            compactions_total: snapshot.compactions_total,
            fsyncs_total: snapshot.fsyncs_total,
            backups_completed_total: snapshot.backups_completed_total,
            backups_failed_total: snapshot.backups_failed_total,
            last_backup_timestamp_ms: snapshot.last_backup_timestamp_ms,
            last_backup_seq: snapshot.last_backup_seq,
            wal_segments_shipped_total: snapshot.wal_segments_shipped_total,
        }
    }
}

/// Autonomous backup status polled from the admin socket.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackupPoll {
    /// True when the worker is running.
    pub enabled: bool,
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
    /// Backup root path, when configured.
    pub backup_root: Option<String>,
    /// Detail of the most recent failure, if any.
    pub last_error: Option<String>,
    /// Sink description, such as `local` or `command`.
    pub sink: String,
}

impl From<WireBackupStatus> for BackupPoll {
    fn from(status: WireBackupStatus) -> Self {
        Self {
            enabled: status.enabled,
            backup_root: status.backup_root,
            last_backup_timestamp_ms: status.last_backup_timestamp_ms,
            last_backup_seq: status.last_backup_seq,
            backups_completed_total: status.backups_completed_total,
            backups_failed_total: status.backups_failed_total,
            wal_segments_shipped_total: status.wal_segments_shipped_total,
            next_full_backup_in_ms: status.next_full_backup_in_ms,
            last_error: status.last_error,
            sink: status.sink,
        }
    }
}

/// Per-level SSTable distribution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LevelStats {
    /// Level number from L0 through L6.
    pub level: u8,
    /// Number of SSTable files in the level.
    pub file_count: usize,
    /// Total bytes of SSTable files in the level.
    pub size_bytes: u64,
}

/// Per-scope activity metrics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeStats {
    /// Identifier of the scope.
    pub scope_id: u32,
    /// Number of SSTable files that hold scope data.
    pub sst_files: u32,
    /// Total writes to the scope.
    pub writes_total: u64,
    /// Total reads from the scope.
    pub reads_total: u64,
    /// Name of the scope.
    pub name: String,
}

/// Rich dashboard data polled from the admin socket.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminPoll {
    /// 99th percentile of the observed fsync latency, in microseconds.
    pub fsync_latency_p99_us: u64,
    /// Number of latency samples behind the percentile.
    pub fsync_latency_samples: u64,
    /// Per-level SSTable distribution for L0 through L6.
    pub levels: Vec<LevelStats>,
    /// Per-scope activity metrics.
    pub scopes: Vec<ScopeStats>,
}

impl From<WireAdminStats> for AdminPoll {
    fn from(stats: WireAdminStats) -> Self {
        let mut levels = empty_level_stats();
        for wire_level in stats.levels {
            if let Some(slot) = levels.get_mut(wire_level.level as usize) {
                slot.file_count = wire_level.file_count as usize;
                slot.size_bytes = wire_level.size_bytes;
            }
        }
        Self {
            levels,
            scopes: stats
                .scopes
                .into_iter()
                .map(scope_stats_from_wire)
                .collect(),
            fsync_latency_p99_us: stats.fsync_latency_p99_us,
            fsync_latency_samples: stats.fsync_latency_samples,
        }
    }
}

/// Combined dashboard snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardSnapshot {
    /// Telemetry counters.
    pub telemetry: TelemetryPoll,
    /// Admin statistics.
    pub admin: AdminPoll,
    /// Backup worker status.
    pub backup: BackupPoll,
}

/// Compute write amplification (writes / reads ratio), returning 0.0 when reads == 0.
pub fn write_amplification(telemetry: &TelemetryPoll) -> f64 {
    if telemetry.reads_total == 0 {
        0.0
    } else {
        telemetry.writes_total as f64 / telemetry.reads_total as f64
    }
}

pub(crate) fn empty_level_stats() -> Vec<LevelStats> {
    (0u8..=6u8)
        .map(|level| LevelStats {
            level,
            file_count: 0,
            size_bytes: 0,
        })
        .collect()
}

fn scope_stats_from_wire(scope: storkv_proto::WireScopeStat) -> ScopeStats {
    ScopeStats {
        scope_id: scope.scope_id,
        name: scope.name,
        writes_total: scope.writes_total,
        reads_total: scope.reads_total,
        sst_files: scope.sst_files,
    }
}
