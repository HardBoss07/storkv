//! Database open recovery: scope reconstruction from the manifest.
//!
//! Scope options and SSTable levels are rebuilt from manifest metadata. WAL
//! replay lives in the `replay` child module.

use crate::manifest::{
    CompactionStyleMode, DurabilityMode, RetentionMode, ScopeMeta, SstFileMeta, StorageModeMode,
};
use crate::scope::{CompactionStyle, Retention, Scope, ScopeOptions, StorageMode};
use crate::sstable::BlockCache;
use crate::vfs::FileSystem;
use crate::wal::Durability;
use bytes::Bytes;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

mod replay;

pub(crate) use replay::{apply_wal_op, replay_wal, scan_wal_segments};

pub(crate) struct LoadedScopes<FS: FileSystem> {
    pub scopes: HashMap<u32, Arc<Scope<FS>>>,
    pub scope_names: HashMap<String, u32>,
    pub created_default: bool,
}

pub(crate) fn load_scopes<FS: FileSystem + 'static>(
    fs: Arc<FS>,
    scopes_dir: &Path,
    manifest_scopes: &HashMap<u32, ScopeMeta>,
    manifest_sst_files: &BTreeMap<(u32, u8, u64), SstFileMeta>,
    manifest_range_tombstones: &BTreeMap<(u32, u64, Bytes), Bytes>,
    block_cache: Arc<BlockCache>,
) -> crate::Result<LoadedScopes<FS>> {
    let mut scopes = HashMap::new();
    let mut scope_names = HashMap::new();

    for (&scope_id, meta) in manifest_scopes {
        let scope_fs = scope_filesystem(fs.clone(), meta);
        let scope = Arc::new(Scope::new(
            scope_id,
            meta.name.clone(),
            scope_options_from_meta(meta),
            scope_fs,
            scopes_dir,
            block_cache.clone(),
        )?);
        if meta.dropped {
            scope.mark_dropped(meta.drop_seq);
        }
        if meta.storage_mode == StorageModeMode::Disk {
            populate_scope_levels(&scope, scope_id, manifest_sst_files);
        }

        scopes.insert(scope_id, scope);
        if !meta.dropped {
            scope_names.insert(meta.name.clone(), scope_id);
        }
    }

    let created_default = if scopes.contains_key(&0) {
        false
    } else {
        let default_scope = Arc::new(Scope::new(
            0,
            "default".to_string(),
            ScopeOptions::default(),
            fs,
            scopes_dir,
            block_cache,
        )?);
        scopes.insert(0, default_scope);
        scope_names.insert("default".to_string(), 0);
        true
    };

    for ((scope_id, seq, start_key), end_key) in manifest_range_tombstones {
        if let Some(scope) = scopes.get(scope_id) {
            scope.record_range_tombstone(start_key.clone(), end_key.clone(), *seq);
        }
    }

    Ok(LoadedScopes {
        scopes,
        scope_names,
        created_default,
    })
}

fn scope_filesystem<FS: FileSystem>(fs: Arc<FS>, meta: &ScopeMeta) -> Arc<FS> {
    if meta.storage_mode == StorageModeMode::Disk {
        return fs;
    }
    fs.memory_view().unwrap_or(fs)
}

fn scope_options_from_meta(meta: &ScopeMeta) -> ScopeOptions {
    let durability = match meta.durability {
        DurabilityMode::StrictFsync => Durability::StrictFsync,
        DurabilityMode::Buffered => Durability::Buffered {
            sync_interval: Duration::from_millis(meta.sync_ms as u64),
        },
    };
    let retention = match meta.retention {
        RetentionMode::Permanent => Retention::Permanent,
        RetentionMode::Ttl => Retention::Ttl {
            default_ttl: Duration::from_millis(meta.default_ttl_ms),
            compaction_style: match meta.compaction_style {
                CompactionStyleMode::Leveled => CompactionStyle::Leveled,
                CompactionStyleMode::Fifo => CompactionStyle::Fifo,
            },
        },
    };
    ScopeOptions {
        durability,
        retention,
        prefix_bloom_len: None,
        storage_mode: storage_mode_from_meta(meta.storage_mode),
        min_ttl: None,
        max_ttl: None,
        allow_permanent: true,
        active_ttl_tracking: false,
    }
}

fn storage_mode_from_meta(mode: StorageModeMode) -> StorageMode {
    match mode {
        StorageModeMode::InMemory => StorageMode::InMemory,
        StorageModeMode::InMemoryPersistent => StorageMode::InMemoryPersistent,
        StorageModeMode::Disk => StorageMode::Disk,
    }
}

fn populate_scope_levels<FS: FileSystem>(
    scope: &Arc<Scope<FS>>,
    scope_id: u32,
    manifest_sst_files: &BTreeMap<(u32, u8, u64), SstFileMeta>,
) {
    let mut levels = scope.sst_levels.write();
    let mut persisted_seq = 0;
    for ((file_scope_id, level, _file_num), sst_meta) in manifest_sst_files {
        if *file_scope_id == scope_id {
            persisted_seq = persisted_seq.max(sst_meta.max_seq);
            if let Some(files) = levels.get_mut(level) {
                files.push(sst_meta.clone());
            }
        }
    }
    // Every record up to the newest sequence in an SSTable is already durable.
    // Set that manifest watermark as the replay floor. A reopen then does not
    // apply flushed WAL records (notably merge operands) a second time. A clean
    // close flushes every scope before the handle drops, so the watermark
    // covers the whole WAL up to that point.
    scope
        .snapshot_seq
        .store(persisted_seq, std::sync::atomic::Ordering::SeqCst);
}
