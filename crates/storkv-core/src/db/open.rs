use crate::db::DbInner;
use crate::manifest::{EditRecord, ManifestManager, VersionEdit};
use crate::mvcc::DefaultMergeOperator;
use crate::options::DbOptions;
use crate::recovery::{load_scopes, replay_wal};
use crate::sstable::BlockCache;
use crate::telemetry::Telemetry;
use crate::vfs::FileSystem;
use crate::wal::WalManager;
use crate::watch::WatchRegistry;
use parking_lot::{Mutex, RwLock};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Open the engine state, replay the WAL, and return the handle.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest, scopes, or WAL cannot be opened.
    pub fn open(fs: Arc<FS>, db_root: PathBuf, options: DbOptions) -> crate::Result<Self> {
        let scopes_dir = db_root.join("scopes");
        let wal_dir = db_root.join("wal");
        fs.create_dir_all(&scopes_dir)?;
        fs.create_dir_all(&wal_dir)?;

        let manifest_mgr = ManifestManager::open(fs.clone(), &db_root)?;
        let (
            active_wal_num,
            manifest_scopes,
            manifest_sst_files,
            manifest_range_tombstones,
            manifest_next_seq,
        ) = {
            let state = manifest_mgr.state();
            (
                state.min_unflushed_wal_num.max(1),
                state.scopes.clone(),
                state.sst_files.clone(),
                state.range_tombstones.clone(),
                state.next_seq,
            )
        };

        let wal_mgr = WalManager::open(fs.clone(), &wal_dir, active_wal_num)?;

        let telemetry = Arc::new(Telemetry::new());
        let block_cache = Arc::new(BlockCache::new(
            options.block_cache_bytes,
            telemetry.clone(),
        ));
        let loaded = load_scopes(
            fs.clone(),
            &scopes_dir,
            &manifest_scopes,
            &manifest_sst_files,
            &manifest_range_tombstones,
            block_cache.clone(),
        )?;
        if loaded.created_default {
            let mut edit = VersionEdit::new();
            edit.add(EditRecord::CreateScope {
                scope_id: 0,
                durability: 0,
                sync_ms: 0,
                retention: 0,
                default_ttl_ms: 0,
                compaction_style: 0,
                storage_mode: 0,
                name: "default".to_string(),
            });
            manifest_mgr.log_edit(edit)?;
        }

        let max_file_num = manifest_sst_files
            .keys()
            .map(|(_, _, file_num)| *file_num)
            .max()
            .unwrap_or(1);
        let mut max_seq = manifest_next_seq;
        for scope in loaded.scopes.values() {
            if scope.options.storage_mode != crate::StorageMode::Disk {
                let snapshot_seq =
                    crate::snapshot::load_scope_snapshot(fs.as_ref(), &db_root, scope)?;
                max_seq = max_seq.max(snapshot_seq);
            }
        }
        let max_seq = replay_wal(fs.clone(), &wal_dir, &loaded.scopes, max_seq);

        Ok(Self {
            fs,
            db_root,
            scopes_dir,
            wal_dir,
            options,
            manifest_mgr,
            wal_mgr,
            scopes: RwLock::new(loaded.scopes),
            scope_names: RwLock::new(loaded.scope_names),
            last_visible_seq: AtomicU64::new(max_seq),
            next_file_num: AtomicU64::new(max_file_num + 1),
            commit_lock: Mutex::new(()),
            watch_registry: WatchRegistry::new(),
            replication_registry: crate::ReplicationRegistry::new(),
            telemetry,
            block_cache,
            active_tx_readers: RwLock::new(Vec::new()),
            merge_operator: Box::new(DefaultMergeOperator),
            compaction_scheduler: Mutex::new(None),
            compaction_cursor: std::sync::atomic::AtomicUsize::new(0),
        })
    }
}
