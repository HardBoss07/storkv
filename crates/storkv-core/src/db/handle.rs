use crate::backup::{
    AutoBackupHandle, BackupConfig, BackupReport, BackupStatus, BackupVerifyReport, RestoreReport,
};
use crate::db::DbInner;
use crate::options::DbOptions;
use crate::telemetry::Telemetry;
use crate::vfs::PosixFs;
use crate::watch::ChangeEvent;
use bytes::Bytes;
use fs2::FileExt;
use parking_lot::Mutex;
use std::fs::File as StdFile;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// Identifier and name of one scope.
#[derive(Debug, Clone)]
pub struct ScopeHandle {
    /// Opaque scope identifier used by data operations.
    pub scope_id: u32,
    /// Unique scope name.
    pub name: String,
}

impl ScopeHandle {
    /// Typed view of the scope identifier.
    pub fn id(&self) -> crate::types::ScopeId {
        crate::types::ScopeId::new(self.scope_id)
    }
}

/// Handle to one open database.
///
/// A `Db` owns the process-wide directory lock and shares the engine state with
/// every clone. Clones are cheap and thread safe. Drop every clone to close the
/// database.
#[derive(Clone)]
pub struct Db {
    inner: Arc<DbInner<PosixFs>>,
    _lock_file: Arc<StdFile>,
    auto_backup: Arc<Mutex<Option<Arc<AutoBackupHandle>>>>,
}

impl Db {
    /// Open or create a database at `path`.
    ///
    /// The call takes an exclusive lock on `<path>/LOCK`. A second process that
    /// opens the same directory receives [`crate::StorKvError::DatabaseLocked`].
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be created, the lock is held,
    /// or recovery fails.
    pub fn open<P: AsRef<Path>>(path: P, options: DbOptions) -> crate::Result<Self> {
        let db_root = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&db_root)?;

        let lock_path = db_root.join("LOCK");
        let lock_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&lock_path)?;

        lock_file
            .try_lock_exclusive()
            .map_err(|_| crate::StorKvError::DatabaseLocked(lock_path.clone()))?;

        let fs = Arc::new(PosixFs);
        let inner = DbInner::open(fs, db_root, options)?;

        Ok(Self {
            inner: Arc::new(inner),
            _lock_file: Arc::new(lock_file),
            auto_backup: Arc::new(Mutex::new(None)),
        })
    }

    /// Create a scope and return its handle.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorKvError::ScopeAlreadyExists`] when the name is taken.
    pub fn create_scope<S: Into<String>>(
        &self,
        name: S,
        options: crate::ScopeOptions,
    ) -> crate::Result<ScopeHandle> {
        self.inner.create_scope(name.into(), options)
    }

    /// Drop a scope and delete its data.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorKvError::ScopeNotFound`] when the name is unknown.
    pub fn drop_scope<S: AsRef<str>>(&self, name: S) -> crate::Result<()> {
        self.inner.drop_scope(name.as_ref())
    }

    /// Begin a transaction at the newest visible sequence number.
    pub fn begin_tx(&self) -> crate::Transaction {
        self.inner.begin_tx()
    }

    /// Begin a transaction pinned to an earlier snapshot.
    ///
    /// Used by adapters (for example Redis `WATCH`) that capture a read
    /// sequence before the transaction starts and later validate it on commit.
    pub fn begin_tx_at(&self, read_seq: u64) -> crate::Transaction {
        self.inner.begin_tx_at(read_seq)
    }

    /// Read one key from the newest visible state.
    ///
    /// # Errors
    ///
    /// Returns an error when a storage read fails.
    pub fn get<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> crate::Result<Option<Bytes>> {
        self.inner.get(scope.scope_id, key.as_ref())
    }

    /// Store one key/value pair.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails.
    pub fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
    ) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.put(scope, key, val)?;
        tx.commit()
    }

    /// Store one key/value pair with a time to live.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails.
    pub fn put_with_ttl<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
        ttl: Duration,
    ) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.put_with_ttl(scope, key, val, ttl)?;
        tx.commit()
    }

    /// Delete one key.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails.
    pub fn delete<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.delete(scope, key)?;
        tx.commit()
    }

    /// Apply a merge operand to one key.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails.
    pub fn merge<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        op: crate::MergeOp,
    ) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.merge(scope, key, op)?;
        tx.commit()
    }

    /// Pin a snapshot at the current sequence number.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest write fails.
    pub fn create_snapshot<S: Into<String>>(&self, name: S) -> crate::Result<()> {
        self.inner.create_snapshot(name.into())
    }

    /// Copy a consistent database image to `dest_dir`.
    ///
    /// # Errors
    ///
    /// Returns an error when the copy fails.
    pub fn checkpoint<P: AsRef<Path>>(&self, dest_dir: P) -> crate::Result<()> {
        self.inner.checkpoint(dest_dir.as_ref())
    }

    /// Release a pinned snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest write fails.
    pub fn unpin_snapshot<S: AsRef<str>>(&self, name: S) -> crate::Result<()> {
        self.inner.unpin_snapshot(name.as_ref())
    }

    /// Delete a half-open key range.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails.
    pub fn delete_range<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        start_key: K,
        end_key: K,
    ) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.delete_range(scope, start_key, end_key)?;
        tx.commit()
    }

    /// Read an ordered key range. `None` bounds are open.
    ///
    /// # Errors
    ///
    /// Returns an error when a storage read fails.
    pub fn scan<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        start_key: Option<K>,
        end_key: Option<K>,
    ) -> crate::Result<Vec<(Bytes, Bytes)>> {
        self.inner.scan(
            scope.scope_id,
            start_key.as_ref().map(|key| key.as_ref()),
            end_key.as_ref().map(|key| key.as_ref()),
            self.inner.last_visible_seq.load(Ordering::Acquire),
        )
    }

    /// Read every key with the given prefix.
    ///
    /// # Errors
    ///
    /// Returns an error when a storage read fails.
    pub fn scan_prefix<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        prefix: K,
    ) -> crate::Result<Vec<(Bytes, Bytes)>> {
        self.inner.scan_prefix(
            scope.scope_id,
            prefix.as_ref(),
            self.inner.last_visible_seq.load(Ordering::Acquire),
        )
    }

    /// Metadata (absolute expiry and sequence) of the visible version of a key.
    pub fn key_meta(
        &self,
        scope: &ScopeHandle,
        key: impl AsRef<[u8]>,
    ) -> crate::Result<Option<crate::scope::KeyMeta>> {
        let read_seq = self.inner.last_visible_seq.load(Ordering::Acquire);
        let scope_state = self
            .inner
            .scopes
            .read()
            .get(&scope.scope_id)
            .cloned()
            .ok_or(crate::StorKvError::ScopeIdNotFound(scope.scope_id))?;
        scope_state.key_meta(
            key.as_ref(),
            crate::types::SequenceNumber::new(read_seq),
            crate::types::TimestampMs::now(),
        )
    }

    /// Remaining TTL in milliseconds.
    ///
    /// `None` means the key does not exist. `Some(0)` means the key is
    /// permanent; any other value is the exact remaining lifetime.
    pub fn key_ttl_remaining_ms(
        &self,
        scope: &ScopeHandle,
        key: impl AsRef<[u8]>,
    ) -> crate::Result<Option<u64>> {
        let Some(meta) = self.key_meta(scope, key)? else {
            return Ok(None);
        };
        if meta.expires_at_ms == 0 {
            return Ok(Some(0));
        }
        let now_ms = crate::types::TimestampMs::now().get();
        Ok(Some(meta.expires_at_ms.saturating_sub(now_ms)))
    }

    /// Return the per-database telemetry counters.
    pub fn telemetry(&self) -> &Telemetry {
        &self.inner.telemetry
    }

    /// Return the shared engine state.
    pub fn inner(&self) -> &Arc<DbInner<PosixFs>> {
        &self.inner
    }

    /// Subscribe to change events.
    pub fn watch(&self) -> crossbeam_channel::Receiver<ChangeEvent> {
        self.inner.watch_registry.subscribe()
    }

    /// Run compaction on every scope immediately.
    ///
    /// # Errors
    ///
    /// Returns an error when compaction or manifest logging fails.
    pub fn compact(&self) -> crate::Result<()> {
        self.inner.run_compaction()
    }

    /// Start the autonomous continuous backup worker.
    ///
    /// Only one worker may run per database. The returned handle shares the
    /// worker with this `Db`; dropping every handle stops it.
    pub fn start_auto_backup(&self, config: BackupConfig) -> crate::Result<Arc<AutoBackupHandle>> {
        let mut slot = self.auto_backup.lock();
        if slot.is_some() {
            return Err(crate::StorKvError::AutoBackupAlreadyRunning);
        }
        let handle = Arc::new(AutoBackupHandle::start(self.inner.clone(), config)?);
        *slot = Some(handle.clone());
        Ok(handle)
    }

    /// Stop the continuous backup worker.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorKvError::AutoBackupNotRunning`] when no worker runs.
    pub fn stop_auto_backup(&self) -> crate::Result<()> {
        let handle = self
            .auto_backup
            .lock()
            .take()
            .ok_or(crate::StorKvError::AutoBackupNotRunning)?;
        handle.shutdown()
    }

    /// Return the live backup status, or `None` when no worker runs.
    pub fn backup_status(&self) -> Option<BackupStatus> {
        self.auto_backup
            .lock()
            .as_ref()
            .map(|handle| handle.status())
    }

    /// Run a full backup immediately. `wait == false` wakes the worker and
    /// returns an empty report; `wait == true` blocks until the generation is
    /// published.
    pub fn trigger_backup(&self, wait: bool) -> crate::Result<BackupReport> {
        let handle = self
            .auto_backup
            .lock()
            .as_ref()
            .cloned()
            .ok_or(crate::StorKvError::AutoBackupNotRunning)?;
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
