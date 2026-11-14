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

pub struct DbInner<FS: FileSystem> {
    pub fs: Arc<FS>,
    pub db_root: PathBuf,
    pub scopes_dir: PathBuf,
    pub wal_dir: PathBuf,
    pub options: DbOptions,
    pub manifest_mgr: ManifestManager<FS>,
    pub wal_mgr: WalManager<FS>,
    pub scopes: RwLock<HashMap<u32, Arc<Scope<FS>>>>,
    pub scope_names: RwLock<HashMap<String, u32>>,
    pub last_visible_seq: AtomicU64,
    pub next_file_num: AtomicU64,
    pub commit_lock: Mutex<()>,
    pub watch_registry: WatchRegistry,
    pub telemetry: Telemetry,
    pub active_tx_readers: RwLock<Vec<u64>>,
    pub merge_operator: Box<dyn MergeOperator>,
}
