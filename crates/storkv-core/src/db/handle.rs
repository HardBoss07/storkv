//! Database and scope handle types.
use crate::backup::AutoBackupHandle;
use crate::db::DbInner;
use crate::options::DbOptions;
use crate::telemetry::Telemetry;
use crate::vfs::{AnyFs, InMemoryFs, PosixFs};
use fs2::FileExt;
use parking_lot::Mutex;
use std::fs::File as StdFile;
use std::path::Path;
use std::sync::Arc;

#[path = "handle_backup.rs"]
mod handle_backup;
#[path = "handle_kv.rs"]
mod handle_kv;
#[path = "handle_kv_options.rs"]
mod handle_kv_options;
#[path = "handle_meta.rs"]
mod handle_meta;
#[path = "handle_scopes.rs"]
mod handle_scopes;
#[path = "handle_streams.rs"]
mod handle_streams;
#[path = "ttl_worker.rs"]
mod ttl_worker;

pub use ttl_worker::TtlWorkerHandle;

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
    inner: Arc<DbInner<AnyFs<PosixFs>>>,
    _lock_file: Option<Arc<StdFile>>,
    auto_backup: Arc<Mutex<Option<Arc<AutoBackupHandle>>>>,
    _ttl_worker: Arc<TtlWorkerHandle>,
}

impl Db {
    /// Open or create a database at `path`.
    ///
    /// The call takes an exclusive lock on `<path>/LOCK`. A second process that
    /// opens the same directory receives `crate::StorkvError::DatabaseLocked`.
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
            .map_err(|_| crate::StorkvError::DatabaseLocked(lock_path.clone()))?;

        let fs = Arc::new(AnyFs::Disk(PosixFs));
        let inner = Arc::new(DbInner::open(fs, db_root, options)?);
        inner.start_compaction_scheduler()?;
        let ttl_worker = Arc::new(TtlWorkerHandle::start(inner.clone()));

        Ok(Self {
            inner,
            _lock_file: Some(Arc::new(lock_file)),
            auto_backup: Arc::new(Mutex::new(None)),
            _ttl_worker: ttl_worker,
        })
    }

    /// Open a database that keeps every file in RAM.
    ///
    /// The database persists nothing. Drop every clone to discard the state.
    ///
    /// # Errors
    ///
    /// Returns an error when the in-memory engine state cannot open.
    pub fn open_in_memory(options: DbOptions) -> crate::Result<Self> {
        let fs = Arc::new(AnyFs::Memory(InMemoryFs::new()));
        let db_root = std::path::PathBuf::from("/storkv-memory");
        let inner = Arc::new(DbInner::open(fs, db_root, options)?);
        inner.start_compaction_scheduler()?;
        let ttl_worker = Arc::new(TtlWorkerHandle::start(inner.clone()));
        Ok(Self {
            inner,
            _lock_file: None,
            auto_backup: Arc::new(Mutex::new(None)),
            _ttl_worker: ttl_worker,
        })
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

    /// Return the per-database telemetry counters.
    pub fn telemetry(&self) -> &Telemetry {
        self.inner.telemetry.as_ref()
    }

    /// Return the shared engine state.
    pub fn inner(&self) -> &Arc<DbInner<AnyFs<PosixFs>>> {
        &self.inner
    }

    /// Return the newest committed sequence number.
    pub fn newest_sequence(&self) -> u64 {
        self.inner
            .last_visible_seq
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Stop the database, unblock every stalled writer, and flush all scopes.
    ///
    /// # Errors
    ///
    /// Returns an error when a flush or snapshot write fails.
    pub fn close(self) -> crate::Result<()> {
        self.inner.shutdown_write_stall();
        self.inner.shutdown_compaction_scheduler();
        self.inner.flush_all()
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            self.inner.shutdown_write_stall();
            self.inner.shutdown_compaction_scheduler();
            let _ = self.inner.flush_all();
        }
    }
}
