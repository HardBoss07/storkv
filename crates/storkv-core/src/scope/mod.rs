use crate::error::Result;
use crate::format::{InternalKey, ValueType};
use crate::manifest::SstFileMeta;
use crate::memtable::{FlushQueue, MemTable};
use crate::sstable::SstReader;
use crate::vfs::FileSystem;
use crate::wal::Durability;
use bytes::Bytes;
use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub const DEFAULT_MAX_WAL_PIN_BYTES: usize = 64 * 1024 * 1024; // 64 MiB
pub const DEFAULT_MAX_WAL_PIN_AGE: Duration = Duration::from_secs(60); // 60s

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    Permanent,
    Ttl {
        default_ttl: Duration,
        compaction_style: CompactionStyle,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionStyle {
    Leveled,
    Fifo,
}

#[derive(Debug, Clone)]
pub struct ScopeOptions {
    pub durability: Durability,
    pub retention: Retention,
    pub prefix_bloom_len: Option<usize>,
}

impl Default for ScopeOptions {
    fn default() -> Self {
        Self {
            durability: Durability::StrictFsync,
            retention: Retention::Permanent,
            prefix_bloom_len: None,
        }
    }
}

pub struct Scope<FS: FileSystem> {
    pub scope_id: u32,
    pub name: String,
    pub options: ScopeOptions,
    pub active_memtable: RwLock<Arc<MemTable>>,
    pub flush_queue: FlushQueue,
    pub sst_levels: RwLock<BTreeMap<u8, Vec<SstFileMeta>>>,
    pub dropped: RwLock<bool>,
    pub drop_seq: AtomicU64,
    pub fs: Arc<FS>,
    pub scope_dir: PathBuf,
}

impl<FS: FileSystem> Scope<FS> {
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
        for lvl in 0..=6 {
            levels.insert(lvl, Vec::new());
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

    pub fn is_dropped(&self) -> bool {
        *self.dropped.read()
    }

    pub fn mark_dropped(&self, drop_seq: u64) {
        let mut d = self.dropped.write();
        *d = true;
        self.drop_seq.store(drop_seq, Ordering::SeqCst);
    }

    pub fn rotate_memtable(&self) -> Arc<MemTable> {
        let old = {
            let mut active = self.active_memtable.write();
            let old = active.clone();
            *active = Arc::new(MemTable::new());
            old
        };
        self.flush_queue.push(old.clone());
        old
    }

    pub fn get_versions(
        &self,
        user_key: &[u8],
        read_seq: u64,
        now_ms: u64,
    ) -> Result<Vec<(ValueType, Bytes)>> {
        if self.is_dropped() {
            let drop_seq = self.drop_seq.load(Ordering::SeqCst);
            if read_seq <= drop_seq {
                return Ok(Vec::new());
            }
        }

        let mut versions = Vec::new();

        // 1. Active MemTable
        {
            let active = self.active_memtable.read();
            let dummy_start = InternalKey::new(
                Bytes::copy_from_slice(user_key),
                u64::MAX,
                ValueType::Value,
                0,
            )
            .encode();
            for (enc_k, val) in active.range_from(dummy_start) {
                if let Some(ikey) = InternalKey::decode(enc_k) {
                    if ikey.user_key.as_ref() != user_key {
                        break;
                    }
                    if ikey.seq <= read_seq {
                        if ikey.expires_at_ms != 0 && ikey.expires_at_ms <= now_ms {
                            continue;
                        }
                        versions.push((ikey.value_type, val));
                        if ikey.value_type != ValueType::MergeOperand {
                            return Ok(versions);
                        }
                    }
                }
            }
        }

        // 2. Immutable MemTables in flush queue
        let queue = self.flush_queue.all();
        for mem in queue.iter().rev() {
            let dummy_start = InternalKey::new(
                Bytes::copy_from_slice(user_key),
                u64::MAX,
                ValueType::Value,
                0,
            )
            .encode();
            for (enc_k, val) in mem.range_from(dummy_start) {
                if let Some(ikey) = InternalKey::decode(enc_k) {
                    if ikey.user_key.as_ref() != user_key {
                        break;
                    }
                    if ikey.seq <= read_seq {
                        if ikey.expires_at_ms != 0 && ikey.expires_at_ms <= now_ms {
                            continue;
                        }
                        versions.push((ikey.value_type, val));
                        if ikey.value_type != ValueType::MergeOperand {
                            return Ok(versions);
                        }
                    }
                }
            }
        }

        // 3. SSTables
        let levels = self.sst_levels.read();
        for level in 0..=6 {
            if let Some(files) = levels.get(&level) {
                for file_meta in files.iter().rev() {
                    if user_key < file_meta.min_key.as_ref()
                        || user_key > file_meta.max_key.as_ref()
                    {
                        continue;
                    }
                    if file_meta.max_expires_at_ms > 0 && file_meta.max_expires_at_ms <= now_ms {
                        continue;
                    }

                    let filename = format!("{:016x}.sst", file_meta.file_num);
                    let sst_path = self.scope_dir.join(filename);
                    if let Ok(mut file) = self.fs.open_file(&sst_path) {
                        if let Ok(reader) = SstReader::open(&mut file) {
                            if !reader.filter.may_contain(user_key) {
                                continue;
                            }
                            if let Ok(entries) = reader.read_all_entries(&mut file) {
                                for (enc_k, val) in entries {
                                    if let Some(ikey) = InternalKey::decode(enc_k) {
                                        if ikey.user_key.as_ref() == user_key
                                            && ikey.seq <= read_seq
                                        {
                                            if ikey.expires_at_ms != 0
                                                && ikey.expires_at_ms <= now_ms
                                            {
                                                continue;
                                            }
                                            versions.push((ikey.value_type, val));
                                            if ikey.value_type != ValueType::MergeOperand {
                                                return Ok(versions);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(versions)
    }

    pub fn get(
        &self,
        user_key: &[u8],
        read_seq: u64,
        now_ms: u64,
    ) -> Result<Option<(ValueType, Bytes)>> {
        let versions = self.get_versions(user_key, read_seq, now_ms)?;
        Ok(versions.into_iter().next())
    }
}
