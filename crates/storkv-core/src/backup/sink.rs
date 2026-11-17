use std::path::Path;

use super::manifest::BackupManifest;

/// Kind of backup sink.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SinkKind {
    /// Backup artifacts are stored in the local filesystem.
    #[default]
    LocalFilesystem,
    /// Backup artifacts are sent to a child process through a pipe.
    CommandPipe,
}

impl SinkKind {
    /// Return the stable sink name.
    pub fn as_str(self) -> &'static str {
        match self {
            SinkKind::LocalFilesystem => "local",
            SinkKind::CommandPipe => "command-pipe",
        }
    }
}

/// Health of a backup sink.
#[derive(Debug, Clone, Default)]
pub struct SinkStatus {
    /// Sink kind.
    pub kind: SinkKind,
    /// Detail of the most recent failure, if any.
    pub last_error: Option<String>,
}

/// Artifacts selected for removal by the retention planner.
#[derive(Debug, Clone, Default)]
pub struct RetentionPlan {
    /// Generation identifiers to delete.
    pub generations: Vec<String>,
    /// WAL segment file numbers to delete.
    pub wal_segments: Vec<u64>,
    /// Number of generations that the plan removes, when known.
    pub generations_removed: Option<usize>,
}

/// Result of a retention prune pass.
#[derive(Debug, Clone, Default)]
pub struct PruneReport {
    /// Number of generations removed.
    pub generations_removed: usize,
    /// Number of WAL segments removed.
    pub wal_segments_removed: usize,
    /// Bytes freed.
    pub bytes_freed: u64,
}

/// Destination for backup generations and incremented WAL segments.
///
/// Implementations must be synchronous and runtime-agnostic so `storkv-core`
/// remains free of async runtimes.
pub trait BackupSink: Send + Sync {
    /// Return the sink kind.
    fn kind(&self) -> SinkKind;

    /// Publish a fully staged, fsynced generation directory.
    fn publish_generation(&self, manifest: &BackupManifest, staged_dir: &Path)
    -> crate::Result<()>;

    /// Ship one sealed WAL segment. Implementations deduplicate by file name.
    fn ship_wal_segment(&self, manifest: &BackupManifest, source: &Path) -> crate::Result<()>;

    /// Remove artifacts selected by the retention planner.
    fn prune(&self, plan: &RetentionPlan) -> crate::Result<PruneReport>;

    /// Return the sink health.
    fn status(&self) -> SinkStatus;
}
