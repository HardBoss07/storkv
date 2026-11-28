//! Autonomous backup worker scheduling.
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;

use crate::db::DbInner;
use crate::error::{Result, StorkvError};
use crate::vfs::{AnyFs, PosixFs};

use super::config::BackupConfig;
use super::sink::PruneReport;

mod controller;

use controller::{BackupController, run_worker};

const SCHEDULER_TICK: Duration = Duration::from_secs(1);

/// Live status of the autonomous backup worker.
#[derive(Debug, Clone, Default)]
pub struct BackupStatus {
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

/// Result of one full backup generation.
#[derive(Debug, Clone, Default)]
pub struct BackupReport {
    /// Sequence number captured by the backup.
    pub sequence: u64,
    /// Number of files in the generation.
    pub files: usize,
    /// Total bytes in the generation.
    pub bytes: u64,
    /// Unique identifier of the new generation.
    pub generation_id: String,
    /// Details of the retention prune pass.
    pub prune: PruneReport,
}

/// Shared handle to a running continuous backup worker.
pub struct AutoBackupHandle {
    controller: Arc<BackupController>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl AutoBackupHandle {
    /// Start the worker thread for a database.
    ///
    /// # Errors
    ///
    /// Returns an error when the configuration is invalid, the backup root
    /// cannot be created, or the worker thread cannot start.
    pub fn start(inner: Arc<DbInner<AnyFs<PosixFs>>>, config: BackupConfig) -> Result<Self> {
        config.validate()?;
        std::fs::create_dir_all(&config.backup_root)?;
        let controller = Arc::new(BackupController::new(inner, config));
        let worker_controller = controller.clone();
        let worker = std::thread::Builder::new()
            .name("storkv-backup".into())
            .spawn(move || run_worker(worker_controller))
            .map_err(|error| {
                StorkvError::InvalidOperation(format!("failed to spawn backup worker: {error}"))
            })?;
        Ok(Self {
            controller,
            worker: Mutex::new(Some(worker)),
        })
    }

    /// Return a point-in-time copy of the worker status.
    pub fn status(&self) -> BackupStatus {
        self.controller.status()
    }

    ///
    /// # Errors
    ///
    /// Returns `StorkvError::AutoBackupNotRunning` when the worker is stopped.
    /// Returns `StorkvError::Io` or `StorkvError::BackupSink` when the backup fails.
    /// Run a full backup synchronously on the calling thread.
    pub fn trigger_now(&self) -> Result<BackupReport> {
        self.controller.run_full_backup()
    }

    /// Wake the worker so its next tick performs a full backup.
    pub fn request_full(&self) {
        let (lock, condvar) = &self.controller.wake;
        *lock.lock() = true;
        condvar.notify_all();
    }

    ///
    /// # Errors
    ///
    /// Never returns an error. The method stops the worker and waits for its thread.
    /// Stop the worker and wait for it to exit.
    pub fn shutdown(&self) -> Result<()> {
        self.stop();
        Ok(())
    }

    fn stop(&self) {
        self.controller.shutdown.store(true, Ordering::SeqCst);
        let (lock, condvar) = &self.controller.wake;
        *lock.lock() = true;
        condvar.notify_all();
        if let Some(worker) = self.worker.lock().take() {
            let _ = worker.join();
        }
    }
}

impl Drop for AutoBackupHandle {
    fn drop(&mut self) {
        self.stop();
    }
}
