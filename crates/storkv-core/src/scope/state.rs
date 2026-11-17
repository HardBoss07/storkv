use crate::error::Result;
use crate::manifest::SstFileMeta;
use crate::memtable::{FlushQueue, MemTable};
use crate::vfs::FileSystem;
use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::options::ScopeOptions;

pub(super) const LEVEL_COUNT: u8 = 7;

/// Runtime state of one scope.
pub struct Scope<FS: FileSystem> {
    /// Opaque scope identifier.
    pub scope_id: u32,
    /// Unique scope name.
    pub name: String,
    /// Scope configuration.
    pub options: ScopeOptions,
    /// Memtable that receives new writes.
    pub active_memtable: RwLock<Arc<MemTable>>,
    /// Immutable memtables that wait for flush.
    pub flush_queue: FlushQueue,
    /// SSTable files per compaction level.
    pub sst_levels: RwLock<BTreeMap<u8, Vec<SstFileMeta>>>,
    /// True after the scope is dropped.
    pub dropped: RwLock<bool>,
    /// Sequence number of the drop.
    pub drop_seq: AtomicU64,
    /// Filesystem backend.
    pub fs: Arc<FS>,
    /// Directory that holds the scope files.
    pub scope_dir: PathBuf,
}

impl<FS: FileSystem> Scope<FS> {
    /// Create a scope and its directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope directory cannot be created.
    pub fn new(
        scope_id: u32,
        name: String,
        options: ScopeOptions,
        fs: Arc<FS>,
        scopes_dir: &Path,
    ) -> Result<Self> {
        let scope_dir = scopes_dir.join(format!("{:08x}", scope_id));
        fs.create_dir_all(&scope_dir)?;

        let mut levels = BTreeMap::new();
        for level in 0..LEVEL_COUNT {
            levels.insert(level, Vec::new());
        }

        Ok(Self {
            scope_id,
            name,
            options,
            active_memtable: RwLock::new(Arc::new(MemTable::new())),
            flush_queue: FlushQueue::new(),
            sst_levels: RwLock::new(levels),
            dropped: RwLock::new(false),
            drop_seq: AtomicU64::new(0),
            fs,
            scope_dir,
        })
    }

    /// Return true when the scope is dropped.
    pub fn is_dropped(&self) -> bool {
        *self.dropped.read()
    }

    /// Mark the scope as dropped at the given sequence number.
    pub fn mark_dropped(&self, drop_seq: u64) {
        let mut dropped = self.dropped.write();
        *dropped = true;
        self.drop_seq.store(drop_seq, Ordering::SeqCst);
    }

    /// Replace the active memtable and queue the old one for flush.
    pub fn rotate_memtable(&self) -> Arc<MemTable> {
        let previous_memtable = {
            let mut active = self.active_memtable.write();
            let previous = active.clone();
            *active = Arc::new(MemTable::new());
            previous
        };
        self.flush_queue.push(previous_memtable.clone());
        previous_memtable
    }
}
