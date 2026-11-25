//! Read-set validation for an optimistic commit.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bytes::Bytes;

use crate::StorkvError;
use crate::db::DbInner;
use crate::format::{InternalKey, ValueType};
use crate::memtable::MemTable;
use crate::mvcc::TransactionContext;
use crate::scope::Scope;
use crate::sstable::SstReader;
use crate::vfs::FileSystem;

impl<FS: FileSystem + 'static> DbInner<FS> {
    pub(super) fn transaction_is_ephemeral(&self, transaction: &TransactionContext) -> bool {
        let scopes = self.scopes.read();
        transaction.write_set.keys().all(|(scope_id, _)| {
            scopes
                .get(scope_id)
                .is_some_and(|scope| scope.options.storage_mode == crate::StorageMode::InMemory)
        })
    }

    pub(super) fn validate_read_set(&self, transaction: &TransactionContext) -> crate::Result<()> {
        // Resolve every scope once and release the registry lock before the
        // validation reads. Validation touches memtables and SSTables, so it
        // must not hold the scope map while doing I/O.
        let targets: Vec<(u32, Bytes, Arc<Scope<FS>>)> = {
            let scopes = self.scopes.read();
            transaction
                .read_set
                .iter()
                .filter_map(|(scope_id, user_key)| {
                    scopes
                        .get(scope_id)
                        .map(|scope| (*scope_id, user_key.clone(), scope.clone()))
                })
                .collect()
        };

        for (scope_id, user_key, scope) in targets {
            if read_set_was_invalidated(&scope, &user_key, transaction.read_seq)? {
                self.telemetry.record_write_conflict();
                return Err(StorkvError::WriteConflict {
                    scope_id,
                    key: user_key.to_vec(),
                });
            }
        }
        Ok(())
    }
}

fn read_set_was_invalidated<FS: FileSystem>(
    scope: &Scope<FS>,
    user_key: &Bytes,
    read_seq: u64,
) -> crate::Result<bool> {
    if scope.is_dropped() && scope.drop_seq.load(Ordering::SeqCst) > read_seq {
        return Ok(true);
    }
    if has_newer_range_tombstone(scope, user_key, read_seq) {
        return Ok(true);
    }
    has_newer_version(scope, user_key, read_seq)
}

fn has_newer_range_tombstone<FS: FileSystem>(
    scope: &Scope<FS>,
    user_key: &Bytes,
    read_seq: u64,
) -> bool {
    let tombstones = scope.range_tombstones.read();
    tombstones.iter().any(|((start, seq), end)| {
        *seq > read_seq && user_key.as_ref() >= start.as_ref() && user_key.as_ref() < end.as_ref()
    })
}

fn has_newer_version<FS: FileSystem>(
    scope: &Scope<FS>,
    user_key: &Bytes,
    read_seq: u64,
) -> crate::Result<bool> {
    let search_start = InternalKey::new(user_key.clone(), u64::MAX, ValueType::Value, 0).encode();

    if memtable_has_newer_version(
        &scope.active_memtable.read(),
        user_key,
        &search_start,
        read_seq,
    ) {
        return Ok(true);
    }
    for memtable in scope.flush_queue.all() {
        if memtable_has_newer_version(&memtable, user_key, &search_start, read_seq) {
            return Ok(true);
        }
    }

    let levels = scope.sst_levels.read();
    for files in levels.values() {
        for file_meta in files {
            // A file whose newest version predates the read cannot invalidate
            // the read set. Skip the expensive SSTable read in that case.
            if file_meta.max_seq <= read_seq {
                continue;
            }
            if !file_may_contain_key(file_meta, user_key) {
                continue;
            }
            if sstable_has_newer_version(scope, file_meta.file_num, user_key, read_seq)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn memtable_has_newer_version(
    memtable: &MemTable,
    user_key: &Bytes,
    search_start: &Bytes,
    read_seq: u64,
) -> bool {
    for (entry_key, _) in memtable.range_from(search_start.clone()) {
        let Some(internal_key) = InternalKey::decode(entry_key) else {
            break;
        };
        if internal_key.user_key != *user_key {
            break;
        }
        if internal_key.seq > read_seq {
            return true;
        }
    }
    false
}

fn sstable_has_newer_version<FS: FileSystem>(
    scope: &Scope<FS>,
    file_num: u64,
    user_key: &Bytes,
    read_seq: u64,
) -> crate::Result<bool> {
    let sst_path = scope.scope_dir.join(format!("{:016x}.sst", file_num));
    let mut file = scope.fs.open_file(&sst_path)?;
    let reader = SstReader::open_cached(
        &mut file,
        &scope.block_cache,
        scope.scope_id,
        file_num,
        true,
    )?;
    if !reader.filter.may_contain(user_key) {
        return Ok(false);
    }
    for (encoded_key, _) in reader.read_all_entries_cached(
        &mut file,
        &scope.block_cache,
        scope.scope_id,
        file_num,
        true,
    )? {
        let Some(internal_key) = InternalKey::decode(encoded_key) else {
            continue;
        };
        if internal_key.user_key == *user_key && internal_key.seq > read_seq {
            return Ok(true);
        }
    }
    Ok(false)
}

fn file_may_contain_key(file_meta: &crate::manifest::SstFileMeta, user_key: &[u8]) -> bool {
    match (
        InternalKey::decode(file_meta.min_key.clone()),
        InternalKey::decode(file_meta.max_key.clone()),
    ) {
        (Some(min_key), Some(max_key)) => {
            user_key >= min_key.user_key.as_ref() && user_key <= max_key.user_key.as_ref()
        }
        _ => true,
    }
}
