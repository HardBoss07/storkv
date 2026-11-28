//! Autonomous continuous backup, sealed-WAL shipping, verification, and
//! point-in-time recovery.
//!
//! The engine is synchronous and runtime-agnostic: callers start a background
//! thread with [`BackupConfig`](crate::backup::BackupConfig) and drive status through `Db` methods.

mod config;
mod manifest;
mod restore;
mod retention;
mod scheduler;
mod sink;
mod sink_local;
mod sink_pipe;
mod verify;
mod wal_index;

pub use config::{BackupCompression, BackupConfig, BackupRetention};
pub use manifest::{
    BACKUP_FORMAT, BACKUP_FORMAT_VERSION, BackupFileEntry, BackupFileKind, BackupKind,
    BackupManifest,
};
pub use restore::{RestoreReport, restore_from_backup, restore_from_config};
pub use retention::{build_plan, load_catalog, max_archived_sequence};
pub use scheduler::{AutoBackupHandle, BackupReport, BackupStatus};
pub use sink::{BackupSink, PruneReport, RetentionPlan, SinkKind, SinkStatus};
pub use sink_local::LocalFilesystemSink;
pub use sink_pipe::{ARCHIVE_MAGIC, ARCHIVE_VERSION, CommandPipeSink, CommandPipeSinkConfig};
pub use verify::{BackupVerificationError, BackupVerifyReport, verify_backup};
pub use wal_index::WalSegmentInfo;
