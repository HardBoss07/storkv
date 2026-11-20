use crate::StorkvError;
use crate::db::DbInner;
use crate::format::{InternalKey, ValueType};
use crate::memtable::MemTable;
use crate::mvcc::MergeOperator;
use crate::scope::Scope;
use crate::sstable::SstReader;
use crate::vfs::FileSystem;
use bytes::Bytes;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Read an ordered range at a read sequence.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::ScopeIdNotFound`] when the scope is unknown.
    pub fn scan(
        &self,
        scope_id: u32,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        read_seq: u64,
    ) -> crate::Result<Vec<(Bytes, Bytes)>> {
        let scope = self
            .scopes
            .read()
            .get(&scope_id)
            .cloned()
            .ok_or(StorkvError::ScopeIdNotFound(scope_id))?;

        if is_dropped_before_read(&scope, read_seq) {
            return Ok(Vec::new());
        }

        let now_ms = crate::types::TimestampMs::now().get();
        let mut state = ScanState::new(read_seq, now_ms);
        self.collect_sstables(&scope, &mut state);
        for memtable in scope.flush_queue.all() {
            collect_memtable_entries(&memtable, &mut state);
        }
        collect_memtable_entries(&scope.active_memtable.read(), &mut state);

        state.into_results(&scope, self.merge_operator.as_ref(), start_key, end_key)
    }

    /// Read every key with the given prefix at a read sequence.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::ScopeIdNotFound`] when the scope is unknown.
    pub fn scan_prefix(
        &self,
        scope_id: u32,
        prefix: &[u8],
        read_seq: u64,
    ) -> crate::Result<Vec<(Bytes, Bytes)>> {
        let mut matches = self.scan(scope_id, Some(prefix), None, read_seq)?;
        matches.retain(|(key, _)| key.starts_with(prefix));
        Ok(matches)
    }
}

struct ScanState {
    read_seq: u64,
    now_ms: u64,
    versions: BTreeMap<Bytes, BTreeMap<u64, (ValueType, Bytes)>>,
}

impl ScanState {
    fn new(read_seq: u64, now_ms: u64) -> Self {
        Self {
            read_seq,
            now_ms,
            versions: BTreeMap::new(),
        }
    }

    fn add_entry(&mut self, encoded_key: Bytes, value: Bytes) {
        let Some(internal_key) = InternalKey::decode(encoded_key) else {
            return;
        };
        if internal_key.seq > self.read_seq {
            return;
        }
        if internal_key.expires_at_ms != 0 && internal_key.expires_at_ms <= self.now_ms {
            return;
        }
        // The scan can see the same entry from more than one source while a
        // memtable is still in the flush queue. Sequence numbers are unique
        // per write, so keep one version per seq.
        self.versions
            .entry(internal_key.user_key)
            .or_default()
            .entry(internal_key.seq)
            .or_insert((internal_key.value_type, value));
    }

    fn into_results<FS: FileSystem>(
        self,
        scope: &Arc<Scope<FS>>,
        merge_operator: &dyn MergeOperator,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
    ) -> crate::Result<Vec<(Bytes, Bytes)>> {
        let mut results = Vec::new();
        for (key, versions) in self.versions {
            if start_key.is_some_and(|start| key.as_ref() < start) {
                continue;
            }
            if end_key.is_some_and(|end| key.as_ref() >= end) {
                continue;
            }

            // Resolve the newest-to-oldest chain exactly like a point read:
            // skip versions hidden by a range tombstone, collect merge
            // operands, and stop at the first value or point tombstone.
            let mut operands = Vec::new();
            let mut base_value = None;
            for (&seq, (value_type, value)) in versions.iter().rev() {
                if *value_type != ValueType::RangeTombstone
                    && scope.is_range_deleted(&key, seq, self.read_seq)
                {
                    continue;
                }
                match value_type {
                    ValueType::MergeOperand => operands.push(value.clone()),
                    ValueType::Value => {
                        base_value = Some(value.clone());
                        break;
                    }
                    ValueType::Tombstone => break,
                    // The tombstone record is not data for this key.
                    // `is_range_deleted` above handles whether it hides older
                    // versions. Skipping it also keeps an empty range
                    // (`start == end`) a no-op for the key at the start bound.
                    ValueType::RangeTombstone => continue,
                }
            }

            let value = if operands.is_empty() {
                match base_value {
                    Some(value) => value,
                    None => continue,
                }
            } else {
                operands.reverse();
                merge_operator.full_merge(&key, base_value.as_deref(), &operands)?
            };
            results.push((key, value));
        }
        Ok(results)
    }
}

fn is_dropped_before_read<FS: FileSystem>(scope: &Scope<FS>, read_seq: u64) -> bool {
    scope.is_dropped() && read_seq >= scope.drop_seq.load(Ordering::SeqCst)
}

impl<FS: FileSystem + 'static> DbInner<FS> {
    fn collect_sstables(&self, scope: &Arc<Scope<FS>>, state: &mut ScanState) {
        let levels = scope.sst_levels.read();
        for level in (0..=6).rev() {
            let Some(files) = levels.get(&level) else {
                continue;
            };
            for file_meta in files {
                self.collect_sstable_entries(scope, file_meta.file_num, state);
            }
        }
    }

    fn collect_sstable_entries(
        &self,
        scope: &Arc<Scope<FS>>,
        file_num: u64,
        state: &mut ScanState,
    ) {
        let sst_path = scope.scope_dir.join(format!("{:016x}.sst", file_num));
        let Ok(mut file) = scope.fs.open_file(&sst_path) else {
            return;
        };
        let Ok(reader) = SstReader::open(&mut file) else {
            return;
        };
        let Ok(entries) = reader.read_all_entries(&mut file) else {
            return;
        };
        for (encoded_key, value) in entries {
            state.add_entry(encoded_key, value);
        }
    }
}

fn collect_memtable_entries(memtable: &MemTable, state: &mut ScanState) {
    for (encoded_key, value) in memtable.iter() {
        state.add_entry(encoded_key, value);
    }
}
