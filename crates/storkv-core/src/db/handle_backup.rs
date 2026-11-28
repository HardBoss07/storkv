//! Checkpoint and continuous-backup operations of a database handle.

use std::path::Path;
use std::sync::Arc;

use crate::backup::{
    AutoBackupHandle, BackupConfig, BackupReport, BackupStatus, BackupVerifyReport, RestoreReport,
};

use super::Db;

impl Db {
    /// Copy a consistent database image to `dest_dir`.
    ///
    /// # Errors
    ///
    /// Returns an error when the copy fails.
    pub fn checkpoint<P: AsRef<Path>>(&self, dest_dir: P) -> crate::Result<()> {
        self.inner.checkpoint(dest_dir.as_ref())
    }

    ///
    /// # Errors
    ///
    /// Returns `StorkvError::AutoBackupAlreadyRunning` when a worker already runs.
    /// Returns `StorkvError::Io` when the backup directory cannot be prepared.
    /// Start the autonomous continuous backup worker.
    ///
    /// Only one worker may run per database. The returned handle shares the
    /// worker with this `Db`; dropping every handle stops it.
    pub fn start_auto_backup(&self, config: BackupConfig) -> crate::Result<Arc<AutoBackupHandle>> {
        let mut slot = self.auto_backup.lock();
        if slot.is_some() {
            return Err(crate::StorkvError::AutoBackupAlreadyRunning);
        }
        let handle = Arc::new(AutoBackupHandle::start(self.inner.clone(), config)?);
        *slot = Some(handle.clone());
        Ok(handle)
    }

    /// Stop the continuous backup worker.
    ///
    /// # Errors
    ///
    /// Returns `crate::StorkvError::AutoBackupNotRunning` when no worker runs.
    pub fn stop_auto_backup(&self) -> crate::Result<()> {
        let handle = self
            .auto_backup
            .lock()
            .take()
            .ok_or(crate::StorkvError::AutoBackupNotRunning)?;
        handle.shutdown()
    }

    /// Return the live backup status, or `None` when no worker runs.
    pub fn backup_status(&self) -> Option<BackupStatus> {
        self.auto_backup
            .lock()
            .as_ref()
            .map(|handle| handle.status())
    }

    ///
    /// # Errors
    ///
    /// Returns `StorkvError::AutoBackupNotRunning` when no worker runs.
    /// Returns `StorkvError::Io` or `StorkvError::BackupSink` when the backup fails.
    /// Run a full backup immediately. `wait == false` wakes the worker and
    /// returns an empty report; `wait == true` blocks until the generation is
    /// published.
    pub fn trigger_backup(&self, wait: bool) -> crate::Result<BackupReport> {
        let handle = self
            .auto_backup
            .lock()
            .as_ref()
            .cloned()
            .ok_or(crate::StorkvError::AutoBackupNotRunning)?;
        if wait {
            handle.trigger_now()
        } else {
            handle.request_full();
            Ok(BackupReport::default())
        }
    }

    /// Restore a database from `backup_root` into `target_data_dir`.
    ///
    /// # Errors
    ///
    /// Returns an error when the target sequence is out of range, verification
    /// fails, or the target directory holds files.
    pub fn restore_from_backup(
        backup_root: &Path,
        target_data_dir: &Path,
        target_seq: Option<u64>,
    ) -> crate::Result<RestoreReport> {
        crate::backup::restore_from_backup(backup_root, target_data_dir, target_seq)
    }

    /// Verify every archived generation in `backup_root`.
    ///
    /// # Errors
    ///
    /// Returns an error when the target sequence is out of range or the catalog
    /// cannot be read.
    pub fn verify_backup(
        backup_root: &Path,
        target_seq: Option<u64>,
    ) -> crate::Result<BackupVerifyReport> {
        crate::backup::verify_backup(backup_root, target_seq)
    }
}
