#![allow(
    clippy::collapsible_if,
    clippy::new_without_default,
    clippy::map_entry,
    clippy::suspicious_open_options
)]

pub mod compaction;
pub mod error;
pub mod format;
pub mod manifest;
pub mod memtable;
pub mod mvcc;
pub mod scope;
pub mod sstable;
pub mod telemetry;
pub mod vfs;
pub mod wal;
pub mod watch;

pub use error::{Result, StorKvError};
pub use mvcc::MergeOp;
pub use scope::{CompactionStyle, Retention, ScopeOptions};
pub use wal::Durability;

use crate::compaction::Compactor;
use crate::format::{InternalKey, MAX_KEY_LEN, MAX_VAL_LEN, ValueType};
use crate::manifest::{EditRecord, ManifestManager, SstFileMeta, VersionEdit};
use crate::mvcc::{DefaultMergeOperator, MergeOperator, TransactionContext, TxMutation};
use crate::scope::{DEFAULT_MAX_WAL_PIN_AGE, DEFAULT_MAX_WAL_PIN_BYTES, Scope};
use crate::sstable::{CompressionType, SstWriter};
use crate::telemetry::Telemetry;
use crate::vfs::{FileSystem, PosixFs};
use crate::wal::{WalBatchFrame, WalManager, WalOp, WalRecovery};
use crate::watch::{ChangeEvent, WatchRegistry};
use bytes::Bytes;
use fs2::FileExt;
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::fs::File as StdFile;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct DbOptions {
    pub max_wal_pin_bytes: usize,
    pub max_wal_pin_age: Duration,
}

impl Default for DbOptions {
    fn default() -> Self {
        Self {
            max_wal_pin_bytes: DEFAULT_MAX_WAL_PIN_BYTES,
            max_wal_pin_age: DEFAULT_MAX_WAL_PIN_AGE,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScopeHandle {
    pub scope_id: u32,
    pub name: String,
}

pub struct Db {
    inner: Arc<DbInner<PosixFs>>,
    _lock_file: StdFile,
}

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

impl Db {
    pub fn open<P: AsRef<Path>>(path: P, options: DbOptions) -> Result<Self> {
        let db_root = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&db_root)?;

        // Acquire exclusive flock on LOCK file
        let lock_path = db_root.join("LOCK");
        let lock_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&lock_path)?;

        lock_file
            .try_lock_exclusive()
            .map_err(|_| StorKvError::DatabaseLocked(lock_path.clone()))?;

        let fs = Arc::new(PosixFs);
        let inner = DbInner::open(fs, db_root, options)?;

        Ok(Self {
            inner: Arc::new(inner),
            _lock_file: lock_file,
        })
    }

    pub fn create_scope<S: Into<String>>(
        &self,
        name: S,
        options: ScopeOptions,
    ) -> Result<ScopeHandle> {
        self.inner.create_scope(name.into(), options)
    }

    pub fn drop_scope<S: AsRef<str>>(&self, name: S) -> Result<()> {
        self.inner.drop_scope(name.as_ref())
    }

    pub fn begin_tx(&self) -> Transaction {
        self.inner.begin_tx()
    }

    pub fn get<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> Result<Option<Bytes>> {
        self.inner.get(scope.scope_id, key.as_ref())
    }

    pub fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
    ) -> Result<()> {
        let mut tx = self.begin_tx();
        tx.put(scope, key, val)?;
        tx.commit()
    }

    pub fn put_with_ttl<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
        ttl: Duration,
    ) -> Result<()> {
        let mut tx = self.begin_tx();
        tx.put_with_ttl(scope, key, val, ttl)?;
        tx.commit()
    }

    pub fn delete<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> Result<()> {
        let mut tx = self.begin_tx();
        tx.delete(scope, key)?;
        tx.commit()
    }

    pub fn merge<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K, op: MergeOp) -> Result<()> {
        let mut tx = self.begin_tx();
        tx.merge(scope, key, op)?;
        tx.commit()
    }

    pub fn create_snapshot<S: Into<String>>(&self, name: S) -> Result<()> {
        self.inner.create_snapshot(name.into())
    }

    pub fn checkpoint<P: AsRef<Path>>(&self, dest_dir: P) -> Result<()> {
        self.inner.checkpoint(dest_dir.as_ref())
    }

    pub fn watch(&self) -> crossbeam_channel::Receiver<ChangeEvent> {
        self.inner.watch_registry.subscribe()
    }

    pub fn compact(&self) -> Result<()> {
        self.inner.run_compaction()
    }
}

impl<FS: FileSystem + 'static> DbInner<FS> {
    pub fn open(fs: Arc<FS>, db_root: PathBuf, options: DbOptions) -> Result<Self> {
        let scopes_dir = db_root.join("scopes");
        let wal_dir = db_root.join("wal");
        fs.create_dir_all(&scopes_dir)?;
        fs.create_dir_all(&wal_dir)?;

        let manifest_mgr = ManifestManager::open(fs.clone(), &db_root)?;
        let (active_wal_num, manifest_scopes, manifest_sst_files, manifest_next_seq) = {
            let s = manifest_mgr.state();
            (
                s.min_unflushed_wal_num.max(1),
                s.scopes.clone(),
                s.sst_files.clone(),
                s.next_seq,
            )
        };

        let wal_mgr = WalManager::open(fs.clone(), &wal_dir, active_wal_num)?;

        let mut scopes = HashMap::new();
        let mut scope_names = HashMap::new();

        // Reconstitute scopes from manifest
        for (&id, meta) in &manifest_scopes {
            let dur = match meta.durability {
                manifest::DurabilityMode::StrictFsync => Durability::StrictFsync,
                manifest::DurabilityMode::Buffered => Durability::Buffered {
                    sync_interval: Duration::from_millis(meta.sync_ms as u64),
                },
            };
            let ret = match meta.retention {
                manifest::RetentionMode::Permanent => Retention::Permanent,
                manifest::RetentionMode::Ttl => Retention::Ttl {
                    default_ttl: Duration::from_millis(meta.default_ttl_ms),
                    compaction_style: match meta.compaction_style {
                        manifest::CompactionStyleMode::Leveled => CompactionStyle::Leveled,
                        manifest::CompactionStyleMode::Fifo => CompactionStyle::Fifo,
                    },
                },
            };
            let scope_opts = ScopeOptions {
                durability: dur,
                retention: ret,
                prefix_bloom_len: None,
            };
            let scope = Arc::new(Scope::new(
                id,
                meta.name.clone(),
                scope_opts,
                fs.clone(),
                &scopes_dir,
            )?);
            if meta.dropped {
                scope.mark_dropped(meta.drop_seq);
            }
            // Populate SST files
            {
                let mut levels = scope.sst_levels.write();
                for ((sid, lvl, _fnum), smeta) in &manifest_sst_files {
                    if *sid == id {
                        if let Some(list) = levels.get_mut(lvl) {
                            list.push(smeta.clone());
                        }
                    }
                }
            }

            scopes.insert(id, scope);
            if !meta.dropped {
                scope_names.insert(meta.name.clone(), id);
            }
        }

        // Ensure default scope (id = 0) exists
        if !scopes.contains_key(&0) {
            let default_opts = ScopeOptions::default();
            let default_scope = Arc::new(Scope::new(
                0,
                "default".into(),
                default_opts,
                fs.clone(),
                &scopes_dir,
            )?);
            scopes.insert(0, default_scope);
            scope_names.insert("default".into(), 0);

            let mut edit = VersionEdit::new();
            edit.add(EditRecord::CreateScope {
                scope_id: 0,
                durability: 0,
                sync_ms: 0,
                retention: 0,
                default_ttl_ms: 0,
                compaction_style: 0,
                name: "default".into(),
            });
            manifest_mgr.log_edit(edit)?;
        }

        let mut max_seq = manifest_next_seq;
        let mut max_file_num = 1;
        for (_, _, fnum) in manifest_sst_files.keys() {
            if *fnum > max_file_num {
                max_file_num = *fnum;
            }
        }

        // Replay WAL segments from active_wal_num
        let wal_files = fs.read_dir(&wal_dir).unwrap_or_default();
        let mut segment_nums: Vec<u64> = Vec::new();
        for path in wal_files {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if let Some(num_str) = name.strip_suffix(".wal") {
                    if let Ok(num) = u64::from_str_radix(num_str, 16) {
                        segment_nums.push(num);
                    }
                }
            }
        }
        segment_nums.sort();

        for (i, num) in segment_nums.iter().enumerate() {
            let path = wal_dir.join(format!("{:016x}.wal", num));
            let is_last = i == segment_nums.len() - 1;
            if let Ok(frames) = WalRecovery::recover_segment(fs.clone(), &path, is_last) {
                for frame in frames {
                    for (op_idx, op) in frame.ops.into_iter().enumerate() {
                        let op_seq = frame.batch_seq + op_idx as u64;
                        if op_seq > max_seq {
                            max_seq = op_seq;
                        }
                        match op {
                            WalOp::Put {
                                scope_id,
                                expires_at_ms,
                                key,
                                val,
                            } => {
                                if let Some(scope) = scopes.get(&scope_id) {
                                    let ikey = InternalKey::new(
                                        key,
                                        op_seq,
                                        ValueType::Value,
                                        expires_at_ms,
                                    );
                                    scope
                                        .active_memtable
                                        .read()
                                        .insert(ikey.encode(), val, op_seq);
                                }
                            }
                            WalOp::Delete { scope_id, key } => {
                                if let Some(scope) = scopes.get(&scope_id) {
                                    let ikey =
                                        InternalKey::new(key, op_seq, ValueType::Tombstone, 0);
                                    scope.active_memtable.read().insert(
                                        ikey.encode(),
                                        Bytes::new(),
                                        op_seq,
                                    );
                                }
                            }
                            WalOp::Merge { scope_id, key, val } => {
                                if let Some(scope) = scopes.get(&scope_id) {
                                    let ikey =
                                        InternalKey::new(key, op_seq, ValueType::MergeOperand, 0);
                                    scope
                                        .active_memtable
                                        .read()
                                        .insert(ikey.encode(), val, op_seq);
                                }
                            }
                            WalOp::DeleteRange {
                                scope_id,
                                start_key,
                                end_key,
                            } => {
                                if let Some(scope) = scopes.get(&scope_id) {
                                    let ikey = InternalKey::new(
                                        start_key,
                                        op_seq,
                                        ValueType::RangeTombstone,
                                        0,
                                    );
                                    scope.active_memtable.read().insert(
                                        ikey.encode(),
                                        end_key,
                                        op_seq,
                                    );
                                }
                            }
                            WalOp::DropScope { scope_id } => {
                                if let Some(scope) = scopes.get(&scope_id) {
                                    scope.mark_dropped(op_seq);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(Self {
            fs,
            db_root,
            scopes_dir,
            wal_dir,
            options,
            manifest_mgr,
            wal_mgr,
            scopes: RwLock::new(scopes),
            scope_names: RwLock::new(scope_names),
            last_visible_seq: AtomicU64::new(max_seq),
            next_file_num: AtomicU64::new(max_file_num + 1),
            commit_lock: Mutex::new(()),
            watch_registry: WatchRegistry::new(),
            telemetry: Telemetry::new(),
            active_tx_readers: RwLock::new(Vec::new()),
            merge_operator: Box::new(DefaultMergeOperator),
        })
    }

    pub fn create_scope(&self, name: String, options: ScopeOptions) -> Result<ScopeHandle> {
        let mut names = self.scope_names.write();
        if names.contains_key(&name) {
            return Err(StorKvError::ScopeAlreadyExists(name));
        }

        let mut scopes = self.scopes.write();
        let scope_id = (scopes.len() as u32) + 1;

        let scope = Arc::new(Scope::new(
            scope_id,
            name.clone(),
            options.clone(),
            self.fs.clone(),
            &self.scopes_dir,
        )?);
        scopes.insert(scope_id, scope);
        names.insert(name.clone(), scope_id);

        let dur_u8 = match options.durability {
            Durability::StrictFsync => 0,
            Durability::Buffered { .. } => 1,
        };
        let sync_ms = match options.durability {
            Durability::Buffered { sync_interval } => sync_interval.as_millis() as u32,
            _ => 0,
        };
        let (ret_u8, ttl_ms, comp_u8) = match options.retention {
            Retention::Permanent => (0, 0, 0),
            Retention::Ttl {
                default_ttl,
                compaction_style,
            } => (
                1,
                default_ttl.as_millis() as u64,
                match compaction_style {
                    CompactionStyle::Leveled => 0,
                    CompactionStyle::Fifo => 1,
                },
            ),
        };

        let mut edit = VersionEdit::new();
        edit.add(EditRecord::CreateScope {
            scope_id,
            durability: dur_u8,
            sync_ms,
            retention: ret_u8,
            default_ttl_ms: ttl_ms,
            compaction_style: comp_u8,
            name: name.clone(),
        });
        self.manifest_mgr.log_edit(edit)?;

        Ok(ScopeHandle { scope_id, name })
    }

    pub fn drop_scope(&self, name: &str) -> Result<()> {
        let scope_id = {
            let mut names = self.scope_names.write();
            names
                .remove(name)
                .ok_or_else(|| StorKvError::ScopeNotFound(name.to_string()))?
        };

        let drop_seq = self.last_visible_seq.fetch_add(1, Ordering::SeqCst) + 1;

        if let Some(scope) = self.scopes.read().get(&scope_id) {
            scope.mark_dropped(drop_seq);
        }

        // Write DropScope to WAL
        let frame = WalBatchFrame::new(drop_seq, vec![WalOp::DropScope { scope_id }]);
        self.wal_mgr.write_batch(&frame, Durability::StrictFsync)?;

        // Write DropScope to MANIFEST
        let mut edit = VersionEdit::new();
        edit.add(EditRecord::DropScope { scope_id, drop_seq });
        self.manifest_mgr.log_edit(edit)?;

        self.watch_registry.broadcast(ChangeEvent::DropScope {
            scope_id,
            seq: drop_seq,
        });

        Ok(())
    }

    pub fn begin_tx(self: &Arc<Self>) -> Transaction<FS> {
        let read_seq = self.last_visible_seq.load(Ordering::Acquire);
        self.active_tx_readers.write().push(read_seq);
        Transaction {
            db: self.clone(),
            ctx: TransactionContext::new(read_seq),
        }
    }

    pub fn get(&self, scope_id: u32, key: &[u8]) -> Result<Option<Bytes>> {
        self.telemetry.record_read();
        let read_seq = self.last_visible_seq.load(Ordering::Acquire);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let scope = self
            .scopes
            .read()
            .get(&scope_id)
            .cloned()
            .ok_or(StorKvError::ScopeIdNotFound(scope_id))?;

        let versions = scope.get_versions(key, read_seq, now_ms)?;
        if versions.is_empty() {
            return Ok(None);
        }

        let mut operands = Vec::new();
        let mut base_val = None;

        for (vt, val) in versions {
            match vt {
                ValueType::MergeOperand => {
                    operands.push(val);
                }
                ValueType::Value => {
                    base_val = Some(val);
                    break;
                }
                ValueType::Tombstone | ValueType::RangeTombstone => {
                    break;
                }
            }
        }

        if operands.is_empty() {
            Ok(base_val)
        } else {
            // Operands are collected newest to oldest; reverse so oldest is applied first
            operands.reverse();
            let merged = self
                .merge_operator
                .full_merge(key, base_val.as_deref(), &operands)?;
            Ok(Some(merged))
        }
    }

    pub fn commit_tx(&self, tx: TransactionContext) -> Result<()> {
        let _commit_guard = self.commit_lock.lock();

        let _now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Optimistic Concurrency Control (OCC) conflict validation:
        // For every key in ReadSet, verify that the latest committed seq <= tx.read_seq
        for &(scope_id, ref user_key) in &tx.read_set {
            if let Some(scope) = self.scopes.read().get(&scope_id) {
                // If scope was dropped after read_seq
                if scope.is_dropped() && scope.drop_seq.load(Ordering::SeqCst) > tx.read_seq {
                    self.telemetry.record_write_conflict();
                    return Err(StorKvError::WriteConflict {
                        scope_id,
                        key: user_key.to_vec(),
                    });
                }

                // Check active memtable and flush queue for newer versions
                let dummy_start =
                    InternalKey::new(user_key.clone(), u64::MAX, ValueType::Value, 0).encode();

                for (entry_key, _) in scope.active_memtable.read().range_from(dummy_start) {
                    let ikey = match InternalKey::decode(entry_key) {
                        Some(k) => k,
                        None => break,
                    };
                    if ikey.user_key != user_key {
                        break;
                    }
                    if ikey.seq > tx.read_seq {
                        self.telemetry.record_write_conflict();
                        return Err(StorKvError::WriteConflict {
                            scope_id,
                            key: user_key.to_vec(),
                        });
                    }
                }
            }
        }

        if tx.write_set.is_empty() {
            return Ok(());
        }

        // Assign contiguous sequence numbers
        let count = tx.write_set.len() as u64;
        let batch_seq = self.last_visible_seq.fetch_add(count, Ordering::SeqCst) + 1;

        let mut ops = Vec::with_capacity(tx.write_set.len());
        let mut overall_durability = Durability::Buffered {
            sync_interval: Duration::from_millis(100),
        };

        for ((scope_id, user_key), mutation) in &tx.write_set {
            if let Some(scope) = self.scopes.read().get(scope_id)
                && scope.options.durability == Durability::StrictFsync
            {
                overall_durability = Durability::StrictFsync;
            }

            match mutation {
                TxMutation::Put { val, expires_at_ms } => {
                    ops.push(WalOp::Put {
                        scope_id: *scope_id,
                        expires_at_ms: *expires_at_ms,
                        key: user_key.clone(),
                        val: val.clone(),
                    });
                }
                TxMutation::Delete => {
                    ops.push(WalOp::Delete {
                        scope_id: *scope_id,
                        key: user_key.clone(),
                    });
                }
                TxMutation::Merge { operand } => {
                    ops.push(WalOp::Merge {
                        scope_id: *scope_id,
                        key: user_key.clone(),
                        val: operand.clone(),
                    });
                }
                TxMutation::DeleteRange { end_key } => {
                    ops.push(WalOp::DeleteRange {
                        scope_id: *scope_id,
                        start_key: user_key.clone(),
                        end_key: end_key.clone(),
                    });
                }
            }
        }

        // Write batch to WAL
        let frame = WalBatchFrame::new(batch_seq, ops);
        self.wal_mgr.write_batch(&frame, overall_durability)?;
        if overall_durability == Durability::StrictFsync {
            self.telemetry.record_fsync();
        }

        // Apply mutations to MemTables and broadcast watch events
        for (idx, (((scope_id, user_key), mutation), _op)) in
            tx.write_set.into_iter().zip(&frame.ops).enumerate()
        {
            let op_seq = batch_seq + idx as u64;
            self.telemetry.record_write();

            if let Some(scope) = self.scopes.read().get(&scope_id) {
                match mutation {
                    TxMutation::Put { val, expires_at_ms } => {
                        let ikey = InternalKey::new(
                            user_key.clone(),
                            op_seq,
                            ValueType::Value,
                            expires_at_ms,
                        );
                        scope
                            .active_memtable
                            .read()
                            .insert(ikey.encode(), val.clone(), op_seq);

                        self.watch_registry.broadcast(ChangeEvent::Put {
                            scope_id,
                            key: user_key,
                            value: val,
                            seq: op_seq,
                        });
                    }
                    TxMutation::Delete => {
                        let ikey =
                            InternalKey::new(user_key.clone(), op_seq, ValueType::Tombstone, 0);
                        scope
                            .active_memtable
                            .read()
                            .insert(ikey.encode(), Bytes::new(), op_seq);

                        self.watch_registry.broadcast(ChangeEvent::Delete {
                            scope_id,
                            key: user_key,
                            seq: op_seq,
                        });
                    }
                    TxMutation::Merge { operand } => {
                        let ikey =
                            InternalKey::new(user_key.clone(), op_seq, ValueType::MergeOperand, 0);
                        scope
                            .active_memtable
                            .read()
                            .insert(ikey.encode(), operand, op_seq);
                    }
                    TxMutation::DeleteRange { end_key } => {
                        let ikey = InternalKey::new(
                            user_key.clone(),
                            op_seq,
                            ValueType::RangeTombstone,
                            0,
                        );
                        scope
                            .active_memtable
                            .read()
                            .insert(ikey.encode(), end_key, op_seq);
                    }
                }

                // Check WAL pinning watermarks (bytes and age)
                let mem_size = scope.active_memtable.read().size_bytes();
                let mem_age = scope.active_memtable.read().created_at().elapsed();
                if mem_size >= self.options.max_wal_pin_bytes
                    || mem_age >= self.options.max_wal_pin_age
                {
                    let _ = self.flush_scope_memtable(scope);
                }
            }
        }

        Ok(())
    }

    pub fn flush_scope_memtable(&self, scope: &Arc<Scope<FS>>) -> Result<()> {
        let mem = scope.rotate_memtable();
        if mem.is_empty() {
            return Ok(());
        }

        let file_num = self.next_file_num.fetch_add(1, Ordering::SeqCst);
        let sst_path = scope.scope_dir.join(format!("{:016x}.sst", file_num));
        let mut file = self.fs.create_file(&sst_path)?;

        let mut writer = SstWriter::new(
            &mut file,
            scope.scope_id,
            0,
            CompressionType::Lz4,
            scope.options.prefix_bloom_len,
        );

        for (enc_k, v) in mem.iter() {
            if let Some(ikey) = InternalKey::decode(enc_k.clone()) {
                let is_tombstone = ikey.value_type == ValueType::Tombstone;
                writer.add(
                    &enc_k,
                    &ikey.user_key,
                    ikey.seq,
                    ikey.expires_at_ms,
                    is_tombstone,
                    &v,
                )?;
            }
        }

        let (_footer, meta, min_key, max_key) = writer.finish()?;

        let sst_meta = SstFileMeta {
            scope_id: scope.scope_id,
            level: 0,
            file_num,
            file_size: meta.raw_key_bytes + meta.raw_val_bytes + 1024,
            min_seq: meta.min_seq,
            max_seq: meta.max_seq,
            max_expires_at_ms: meta.max_expires_at_ms,
            min_key,
            max_key,
        };

        // Manifest log
        let mut edit = VersionEdit::new();
        edit.add(EditRecord::AddSstFile(sst_meta.clone()));
        self.manifest_mgr.log_edit(edit)?;

        scope
            .sst_levels
            .write()
            .entry(0)
            .or_default()
            .push(sst_meta);
        let _ = scope.flush_queue.pop_oldest();

        Ok(())
    }

    pub fn create_snapshot(&self, name: String) -> Result<()> {
        let seq = self.last_visible_seq.load(Ordering::Acquire);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut edit = VersionEdit::new();
        edit.add(EditRecord::PinSnapshot {
            seq,
            created_at_ms: now_ms,
            name: name.clone(),
        });
        self.manifest_mgr.log_edit(edit)
    }

    pub fn checkpoint(&self, dest_dir: &Path) -> Result<()> {
        self.fs.create_dir_all(dest_dir)?;

        // Flush all active scopes
        let scopes = self.scopes.read().clone();
        for scope in scopes.values() {
            let _ = self.flush_scope_memtable(scope);
        }

        // Hard-link all live SSTables into dest_dir/scopes/<id>/
        let dest_scopes = dest_dir.join("scopes");
        self.fs.create_dir_all(&dest_scopes)?;

        let state = self.manifest_mgr.state();
        for (scope_id, _lvl, file_num) in state.sst_files.keys() {
            let scope_sub = dest_scopes.join(format!("{:08x}", scope_id));
            self.fs.create_dir_all(&scope_sub)?;

            let filename = format!("{:016x}.sst", file_num);
            let src = self
                .scopes_dir
                .join(format!("{:08x}", scope_id))
                .join(&filename);
            let dst = scope_sub.join(&filename);
            if self.fs.exists(&src) {
                let _ = self.fs.hard_link(&src, &dst);
            }
        }

        // Copy CURRENT and active MANIFEST
        let cur_src = self.db_root.join("CURRENT");
        let cur_dst = dest_dir.join("CURRENT");
        if let Ok(mut sf) = self.fs.open_file(&cur_src) {
            let mut buf = Vec::new();
            sf.read_to_end(&mut buf)?;
            let mut df = self.fs.create_file(&cur_dst)?;
            df.write_all(&buf)?;
            df.sync_all()?;

            let manifest_name = String::from_utf8_lossy(&buf).trim().to_string();
            let mnf_src = self.db_root.join(&manifest_name);
            let mnf_dst = dest_dir.join(&manifest_name);
            if let Ok(mut msf) = self.fs.open_file(&mnf_src) {
                let mut mbuf = Vec::new();
                msf.read_to_end(&mut mbuf)?;
                let mut mdf = self.fs.create_file(&mnf_dst)?;
                mdf.write_all(&mbuf)?;
                mdf.sync_all()?;
            }
        }

        // Copy WAL files
        let dest_wal = dest_dir.join("wal");
        self.fs.create_dir_all(&dest_wal)?;
        let wal_files = self.fs.read_dir(&self.wal_dir).unwrap_or_default();
        for p in wal_files {
            if let Some(fname) = p.file_name() {
                let dst_w = dest_wal.join(fname);
                if let Ok(mut wf) = self.fs.open_file(&p) {
                    let mut wbuf = Vec::new();
                    wf.read_to_end(&mut wbuf)?;
                    let mut dwf = self.fs.create_file(&dst_w)?;
                    dwf.write_all(&wbuf)?;
                    dwf.sync_all()?;
                }
            }
        }

        self.fs.sync_dir(dest_dir)?;
        Ok(())
    }

    pub fn gc_safe_seq(&self) -> u64 {
        let readers = self.active_tx_readers.read();
        let oldest_tx = readers.iter().min().copied().unwrap_or(u64::MAX);

        let state = self.manifest_mgr.state();
        let oldest_snap = state
            .pinned_snapshots
            .values()
            .map(|s| s.seq)
            .min()
            .unwrap_or(u64::MAX);

        oldest_tx
            .min(oldest_snap)
            .min(self.last_visible_seq.load(Ordering::Acquire))
    }

    pub fn run_compaction(&self) -> Result<()> {
        self.telemetry.record_compaction();
        let gc_safe = self.gc_safe_seq();
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let scopes = self.scopes.read().clone();
        for scope in scopes.values() {
            if let Some(edit) =
                Compactor::compact_scope(scope, gc_safe, &self.next_file_num, now_ms)?
            {
                self.manifest_mgr.log_edit(edit)?;
            }
        }

        Ok(())
    }
}

pub struct Transaction<FS: FileSystem + 'static = PosixFs> {
    db: Arc<DbInner<FS>>,
    ctx: TransactionContext,
}

impl<FS: FileSystem + 'static> Transaction<FS> {
    pub fn get<K: AsRef<[u8]>>(&mut self, scope: &ScopeHandle, key: K) -> Result<Option<Bytes>> {
        self.ctx
            .record_read(scope.scope_id, Bytes::copy_from_slice(key.as_ref()));

        // Inspect local WriteSet first (Read-Your-Own-Writes)
        if let Some(mutation) = self.ctx.get_local(scope.scope_id, key.as_ref()) {
            match mutation {
                TxMutation::Put { val, .. } => return Ok(Some(val.clone())),
                TxMutation::Delete => return Ok(None),
                TxMutation::Merge { operand } => {
                    let base = self.db.get(scope.scope_id, key.as_ref())?;
                    let merged = self.db.merge_operator.full_merge(
                        key.as_ref(),
                        base.as_deref(),
                        std::slice::from_ref(operand),
                    )?;
                    return Ok(Some(merged));
                }
                TxMutation::DeleteRange { .. } => return Ok(None),
            }
        }

        self.db.get(scope.scope_id, key.as_ref())
    }

    pub fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &mut self,
        scope: &ScopeHandle,
        key: K,
        val: V,
    ) -> Result<()> {
        let k = key.as_ref();
        let v = val.as_ref();
        if k.len() > MAX_KEY_LEN {
            return Err(StorKvError::KeyTooLarge {
                max: MAX_KEY_LEN,
                actual: k.len(),
            });
        }
        if v.len() > MAX_VAL_LEN {
            return Err(StorKvError::ValueTooLarge {
                max: MAX_VAL_LEN,
                actual: v.len(),
            });
        }
        self.ctx.put(
            scope.scope_id,
            Bytes::copy_from_slice(k),
            Bytes::copy_from_slice(v),
            0,
        );
        Ok(())
    }

    pub fn put_with_ttl<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &mut self,
        scope: &ScopeHandle,
        key: K,
        val: V,
        ttl: Duration,
    ) -> Result<()> {
        let k = key.as_ref();
        let v = val.as_ref();
        if k.len() > MAX_KEY_LEN {
            return Err(StorKvError::KeyTooLarge {
                max: MAX_KEY_LEN,
                actual: k.len(),
            });
        }
        if v.len() > MAX_VAL_LEN {
            return Err(StorKvError::ValueTooLarge {
                max: MAX_VAL_LEN,
                actual: v.len(),
            });
        }
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let expires_at_ms = now_ms + ttl.as_millis() as u64;

        self.ctx.put(
            scope.scope_id,
            Bytes::copy_from_slice(k),
            Bytes::copy_from_slice(v),
            expires_at_ms,
        );
        Ok(())
    }

    pub fn delete<K: AsRef<[u8]>>(&mut self, scope: &ScopeHandle, key: K) -> Result<()> {
        let k = key.as_ref();
        if k.len() > MAX_KEY_LEN {
            return Err(StorKvError::KeyTooLarge {
                max: MAX_KEY_LEN,
                actual: k.len(),
            });
        }
        self.ctx.delete(scope.scope_id, Bytes::copy_from_slice(k));
        Ok(())
    }

    pub fn merge<K: AsRef<[u8]>>(
        &mut self,
        scope: &ScopeHandle,
        key: K,
        op: MergeOp,
    ) -> Result<()> {
        let k = key.as_ref();
        let operand = match op {
            MergeOp::CounterAdd(v) => Bytes::from(v.to_string()),
            MergeOp::Custom(b) => b,
        };
        self.ctx
            .merge(scope.scope_id, Bytes::copy_from_slice(k), operand);
        Ok(())
    }

    pub fn commit(mut self) -> Result<()> {
        let read_seq = self.ctx.read_seq;
        let ctx = std::mem::replace(&mut self.ctx, TransactionContext::new(0));
        let res = self.db.commit_tx(ctx);
        let mut readers = self.db.active_tx_readers.write();
        if let Some(pos) = readers.iter().position(|&x| x == read_seq) {
            readers.remove(pos);
        }
        res
    }
}

impl<FS: FileSystem + 'static> Drop for Transaction<FS> {
    fn drop(&mut self) {
        let mut readers = self.db.active_tx_readers.write();
        if let Some(pos) = readers.iter().position(|&x| x == self.ctx.read_seq) {
            readers.remove(pos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_engine_integration() {
        let temp = std::env::temp_dir().join(format!("storkv_test_engine_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);

        let db = Db::open(&temp, DbOptions::default()).unwrap();

        let sessions = db
            .create_scope(
                "sessions",
                ScopeOptions {
                    durability: Durability::Buffered {
                        sync_interval: Duration::from_millis(100),
                    },
                    retention: Retention::Ttl {
                        default_ttl: Duration::from_secs(900),
                        compaction_style: CompactionStyle::Fifo,
                    },
                    prefix_bloom_len: Some(8),
                },
            )
            .unwrap();

        let accounts = db
            .create_scope(
                "accounts",
                ScopeOptions {
                    durability: Durability::StrictFsync,
                    retention: Retention::Permanent,
                    prefix_bloom_len: None,
                },
            )
            .unwrap();

        // Transaction write
        let mut tx = db.begin_tx();
        tx.put(&accounts, b"user:42:balance", b"5000").unwrap();
        tx.put_with_ttl(
            &sessions,
            b"sess:abc987",
            b"user:42",
            Duration::from_secs(1800),
        )
        .unwrap();
        tx.commit().unwrap();

        // Read back
        let mut read_tx = db.begin_tx();
        let val = read_tx.get(&sessions, b"sess:abc987").unwrap().unwrap();
        assert_eq!(val.as_ref(), b"user:42");

        // Merge operation
        db.merge(&accounts, b"user:42:login_count", MergeOp::CounterAdd(1))
            .unwrap();
        db.merge(&accounts, b"user:42:login_count", MergeOp::CounterAdd(2))
            .unwrap();
        let count = db.get(&accounts, b"user:42:login_count").unwrap().unwrap();
        assert_eq!(count.as_ref(), b"3");

        // Snapshots and checkpoints
        db.create_snapshot("snap1").unwrap();
        let cp_dir = temp.join("checkpoints").join("cp1");
        db.checkpoint(&cp_dir).unwrap();
        assert!(cp_dir.exists());

        let mut tx1 = db.begin_tx();
        let mut tx2 = db.begin_tx();

        let _ = tx1.get(&accounts, b"user:42:balance").unwrap();
        let _ = tx2.get(&accounts, b"user:42:balance").unwrap();

        tx1.put(&accounts, b"user:42:balance", b"6000").unwrap();
        tx1.commit().unwrap();

        tx2.put(&accounts, b"user:42:balance", b"7000").unwrap();
        let conflict_res = tx2.commit();
        assert!(matches!(
            conflict_res,
            Err(StorKvError::WriteConflict { .. })
        ));

        drop(db);
        let _ = std::fs::remove_dir_all(&temp);
    }
}
