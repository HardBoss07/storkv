use crate::error::Result;
use crate::manifest::SstFileMeta;
use crate::memtable::{FlushQueue, MemTable};
use crate::vfs::FileSystem;
use bytes::Bytes;
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
    /// Sequence number covered by the recovery snapshot.
    pub snapshot_seq: AtomicU64,
    /// Range tombstones keyed by start key and sequence number.
    ///
    /// The map value holds the exclusive end key. Point reads consult this
    /// index because a range tombstone has no entry at the covered keys.
    pub range_tombstones: RwLock<BTreeMap<(Bytes, u64), Bytes>>,
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
            snapshot_seq: AtomicU64::new(0),
            range_tombstones: RwLock::new(BTreeMap::new()),
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

    /// Record a range tombstone in the in-memory index.
    ///
    /// The method drops an older tombstone that the new range contains. This
    /// makes repeated whole-scope deletes cheap. A duplicate tombstone is
    /// ignored, so WAL replay is safe.
    pub fn record_range_tombstone(&self, start_key: Bytes, end_key: Bytes, seq: u64) {
        let mut tombstones = self.range_tombstones.write();
        tombstones.retain(|(old_start, old_seq), old_end| {
            !(*old_seq <= seq
                && old_start.as_ref() >= start_key.as_ref()
                && old_end.as_ref() <= end_key.as_ref())
        });
        tombstones.entry((start_key, seq)).or_insert(end_key);
    }

    /// Return true when a range tombstone hides a key version.
    ///
    /// The tombstone must be visible at `read_seq` and must be at or after the
    /// version sequence. The key must lie in the half-open tombstone range.
    pub fn is_range_deleted(&self, key: &[u8], version_seq: u64, read_seq: u64) -> bool {
        let tombstones = self.range_tombstones.read();
        tombstones.iter().any(|((start, seq), end)| {
            *seq <= read_seq && *seq >= version_seq && key >= start.as_ref() && key < end.as_ref()
        })
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
