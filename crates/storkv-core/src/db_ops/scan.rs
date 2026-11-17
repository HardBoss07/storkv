use crate::StorKvError;
use crate::db::DbInner;
use crate::format::{InternalKey, ValueType};
use crate::memtable::MemTable;
use crate::scope::Scope;
use crate::sstable::SstReader;
use crate::types::TimestampMs;
use crate::vfs::FileSystem;
use bytes::Bytes;
use std::sync::Arc;
use std::sync::atomic::Ordering;

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Read an ordered range at a read sequence.
    ///
    /// # Errors
    ///
    /// Returns [`StorKvError::ScopeIdNotFound`] when the scope is unknown.
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
            .ok_or(StorKvError::ScopeIdNotFound(scope_id))?;

        if is_dropped_before_read(&scope, read_seq) {
            return Ok(Vec::new());
        }

        let mut state = ScanState::new(read_seq, TimestampMs::now().get());
        self.collect_sstables(&scope, &mut state);
        for memtable in scope.flush_queue.all() {
            collect_memtable_entries(&memtable, &mut state);
        }
        collect_memtable_entries(&scope.active_memtable.read(), &mut state);

        Ok(state.into_results(start_key, end_key))
    }

    /// Read every key with the given prefix at a read sequence.
    ///
    /// # Errors
    ///
    /// Returns [`StorKvError::ScopeIdNotFound`] when the scope is unknown.
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

pub(super) struct ScanState {
    read_seq: u64,
    now_ms: u64,
    range_tombstones: Vec<(Bytes, Bytes, u64)>,
    collected: std::collections::BTreeMap<Bytes, (ValueType, Bytes, u64)>,
}

impl ScanState {
    pub(super) fn new(read_seq: u64, now_ms: u64) -> Self {
        Self {
            read_seq,
            now_ms,
            range_tombstones: Vec::new(),
            collected: std::collections::BTreeMap::new(),
        }
    }

    pub(super) fn add_entry(&mut self, encoded_key: Bytes, value: Bytes) {
        let Some(internal_key) = InternalKey::decode(encoded_key) else {
            return;
        };
        if internal_key.seq > self.read_seq {
            return;
        }
        if internal_key.expires_at_ms != 0 && internal_key.expires_at_ms <= self.now_ms {
            return;
        }
        if internal_key.value_type == ValueType::RangeTombstone {
            self.range_tombstones
                .push((internal_key.user_key, value, internal_key.seq));
        } else {
            if let Some((_, _, existing_seq)) = self.collected.get(&internal_key.user_key) {
                if *existing_seq >= internal_key.seq {
                    return;
                }
            }
            self.collected.insert(
                internal_key.user_key,
                (internal_key.value_type, value, internal_key.seq),
            );
        }
    }

    fn into_results(self, start_key: Option<&[u8]>, end_key: Option<&[u8]>) -> Vec<(Bytes, Bytes)> {
        let mut results = Vec::new();
        'key_loop: for (key, (value_type, value, seq)) in self.collected {
            if start_key.is_some_and(|start| key.as_ref() < start) {
                continue;
            }
            if end_key.is_some_and(|end| key.as_ref() >= end) {
                continue;
            }
            if value_type != ValueType::Value {
                continue;
            }
            if is_range_deleted(&self.range_tombstones, &key, seq) {
                continue 'key_loop;
            }
            results.push((key, value));
        }
        results
    }
}

fn is_range_deleted(tombstones: &[(Bytes, Bytes, u64)], key: &Bytes, seq: u64) -> bool {
    tombstones
        .iter()
        .any(|(tombstone_start, tombstone_end, tombstone_seq)| {
            *tombstone_seq >= seq
                && key.as_ref() >= tombstone_start.as_ref()
                && key.as_ref() < tombstone_end.as_ref()
        })
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
        let Ok(mut file) = self.fs.open_file(&sst_path) else {
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
