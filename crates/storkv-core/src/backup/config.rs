//! Backup configuration and retention policy.
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::backup::sink::BackupSink;
use crate::backup::sink_local::LocalFilesystemSink;
use crate::backup::sink_pipe::{CommandPipeSink, CommandPipeSinkConfig};

const DEFAULT_MAX_FULL_BACKUPS: usize = 7;
const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY: u64 = 24 * SECONDS_PER_HOUR;
const DEFAULT_MAX_WAL_AGE_SECS: u64 = 7 * SECONDS_PER_DAY;
const DEFAULT_FULL_BACKUP_INTERVAL_SECS: u64 = SECONDS_PER_HOUR;

/// How the auto-backup scheduler behaves and where artifacts are written.
pub struct BackupConfig {
    /// Ship sealed WAL segments between full backups when true.
    pub ship_sealed_wal: bool,
    /// Time between full backup generations.
    pub full_backup_interval: Duration,
    /// Directory that stores backup artifacts.
    pub backup_root: PathBuf,
    /// Retention limits for full generations and WAL segments.
    pub retention: BackupRetention,
    /// Compression codec for archive and manifest payloads.
    pub compression: BackupCompression,
    /// Destination for generations and WAL segments.
    pub sink: Arc<dyn BackupSink>,
}

/// Retention limits applied by the backup prune pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupRetention {
    /// Maximum number of full generations to keep.
    pub max_full_backups: usize,
    /// Maximum age of a WAL segment before it is pruned.
    pub max_wal_age: Duration,
}

/// Compression codec used for backup artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupCompression {
    /// Store artifacts without compression.
    None,
    /// Compress with Zstandard.
    Zstd,
    /// Compress with LZ4.
    Lz4,
}

impl BackupCompression {
    /// Return the stable codec name.
    pub fn as_str(self) -> &'static str {
        match self {
            BackupCompression::None => "none",
            BackupCompression::Zstd => "zstd",
            BackupCompression::Lz4 => "lz4",
        }
    }

    /// Parse a codec name.
    ///
    /// # Errors
    ///
    /// Returns `crate::StorkvError::InvalidOperation` for an unknown name.
    pub fn parse(value: &str) -> crate::Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "none" => Ok(BackupCompression::None),
            "zstd" => Ok(BackupCompression::Zstd),
            "lz4" => Ok(BackupCompression::Lz4),
            other => Err(crate::StorkvError::InvalidOperation(format!(
                "unsupported backup compression `{other}` (expected none, zstd, or lz4)"
            ))),
        }
    }
}

impl Default for BackupRetention {
    fn default() -> Self {
        Self {
            max_full_backups: DEFAULT_MAX_FULL_BACKUPS,
            max_wal_age: Duration::from_secs(DEFAULT_MAX_WAL_AGE_SECS),
        }
    }
}

impl BackupConfig {
    /// Build a configuration that writes to a local directory.
    pub fn local(backup_root: impl Into<PathBuf>) -> Self {
        let backup_root = backup_root.into();
        let sink = Arc::new(LocalFilesystemSink::new(backup_root.clone()));
        Self::with_sink(backup_root, sink)
    }

    /// Build a configuration that sends archives to a child process.
    ///
    /// # Errors
    ///
    /// Returns an error when the command configuration is invalid.
    pub fn command(
        backup_root: impl Into<PathBuf>,
        command: CommandPipeSinkConfig,
    ) -> crate::Result<Self> {
        let backup_root = backup_root.into();
        let compression = command.compression;
        let sink = Arc::new(CommandPipeSink::new(command)?);
        let mut config = Self::with_sink(backup_root, sink);
        config.compression = compression;
        Ok(config)
    }

    fn with_sink(backup_root: PathBuf, sink: Arc<dyn BackupSink>) -> Self {
        Self {
            backup_root,
            full_backup_interval: Duration::from_secs(DEFAULT_FULL_BACKUP_INTERVAL_SECS),
            ship_sealed_wal: true,
            retention: BackupRetention::default(),
            compression: BackupCompression::None,
            sink,
        }
    }

    /// Override the full backup interval.
    pub fn with_full_backup_interval(mut self, interval: Duration) -> Self {
        self.full_backup_interval = interval;
        self
    }

    /// Enable or disable sealed WAL shipping.
    pub fn with_wal_shipping(mut self, ship_sealed_wal: bool) -> Self {
        self.ship_sealed_wal = ship_sealed_wal;
        self
    }

    /// Override the retention limits.
    pub fn with_retention(mut self, retention: BackupRetention) -> Self {
        self.retention = retention;
        self
    }

    /// Override the compression codec.
    pub fn with_compression(mut self, compression: BackupCompression) -> Self {
        self.compression = compression;
        self
    }

    pub(crate) fn validate(&self) -> crate::Result<()> {
        if self.full_backup_interval.is_zero() {
            return Err(crate::StorkvError::InvalidOperation(
                "backup interval must be greater than zero".into(),
            ));
        }
        if self.retention.max_full_backups == 0 {
            return Err(crate::StorkvError::InvalidOperation(
                "backup retention must keep at least one full generation".into(),
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for BackupConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackupConfig")
            .field("backup_root", &self.backup_root)
            .field("full_backup_interval", &self.full_backup_interval)
            .field("ship_sealed_wal", &self.ship_sealed_wal)
            .field("retention", &self.retention)
            .field("compression", &self.compression)
            .finish_non_exhaustive()
    }
}
