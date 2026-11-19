use crate::manifest::ManifestManager;
use crate::mvcc::MergeOperator;
use crate::options::DbOptions;
use crate::scope::Scope;
use crate::telemetry::Telemetry;
use crate::vfs::FileSystem;
use crate::wal::WalManager;
use crate::watch::WatchRegistry;
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

/// Default maximum number of scopes in one database.
pub const MAX_SCOPES: usize = 1024;

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
    pub telemetry: Telemetry,
    /// Read sequence numbers of open transactions.
    pub active_tx_readers: RwLock<Vec<u64>>,
    /// Merge operator for merge writes.
    pub merge_operator: Box<dyn MergeOperator>,
}
