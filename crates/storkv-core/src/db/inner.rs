use crate::compaction::CompactionScheduler;
use crate::manifest::ManifestManager;
use crate::mvcc::MergeOperator;
use crate::options::DbOptions;
use crate::scope::Scope;
use crate::sstable::BlockCache;
use crate::telemetry::Telemetry;
use crate::vfs::FileSystem;
use crate::wal::WalManager;
use crate::watch::WatchRegistry;
use parking_lot::{Condvar, Mutex, RwLock};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};

/// Default maximum number of scopes in one database.
pub const MAX_SCOPES: usize = 1024;

/// Coordination state for foreground write backpressure.
pub(crate) struct WriteStall {
    pub(crate) shutdown: AtomicBool,
    pub(crate) state: Mutex<()>,
    pub(crate) wake: Condvar,
}

impl WriteStall {
    pub(crate) fn new() -> Self {
        Self {
            shutdown: AtomicBool::new(false),
            state: Mutex::new(()),
            wake: Condvar::new(),
        }
    }
}

/// Shared inner state of one database.
pub struct DbInner<FS: FileSystem> {
    /// Filesystem backend.
    pub fs: Arc<FS>,
    /// Database root directory.
    pub db_root: PathBuf,
    /// Directory that holds scope subdirectories.
    pub scopes_dir: PathBuf,
    /// Directory that holds WAL segments.
    pub wal_dir: PathBuf,
    /// Database options.
    pub options: DbOptions,
    /// Manifest reader and writer.
    pub manifest_mgr: ManifestManager<FS>,
    /// Active WAL manager.
    pub wal_mgr: WalManager<FS>,
    /// Live scopes keyed by identifier.
    pub scopes: RwLock<HashMap<u32, Arc<Scope<FS>>>>,
    /// Scope identifiers keyed by name.
    pub scope_names: RwLock<HashMap<String, u32>>,
    /// Newest committed sequence number.
    pub last_visible_seq: AtomicU64,
    /// Next file number to allocate.
    pub next_file_num: AtomicU64,
    /// Serializes commit operations.
    pub commit_lock: Mutex<()>,
    /// Change event subscribers.
    pub watch_registry: WatchRegistry,
    /// Replication frame subscribers.
    pub replication_registry: crate::ReplicationRegistry,
    /// Runtime counters.
    pub telemetry: Arc<Telemetry>,
    /// Shared SSTable block cache.
    pub block_cache: Arc<BlockCache>,
    /// Read sequence numbers of open transactions.
    pub active_tx_readers: RwLock<Vec<u64>>,
    /// Merge operator for merge writes.
    pub merge_operator: Box<dyn MergeOperator>,
    /// Autonomous background compaction scheduler (or `None` if disabled).
    pub compaction_scheduler: Mutex<Option<CompactionScheduler<FS>>>,
    /// Rotation cursor that gives every scope a fair compaction turn.
    pub compaction_cursor: std::sync::atomic::AtomicUsize,
    /// Approximate WAL bytes that no MemTable flush has drained yet.
    pub(crate) unflushed_wal_bytes: AtomicU64,
    /// Write-stall coordination state.
    pub(crate) write_stall: WriteStall,
}
