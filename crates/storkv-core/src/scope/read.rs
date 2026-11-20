use crate::error::Result;
use crate::format::{InternalKey, ValueType};
use crate::manifest::SstFileMeta;
use crate::memtable::MemTable;
use crate::sstable::SstReader;
use crate::types::{SequenceNumber, TimestampMs};
use crate::vfs::FileSystem;
use bytes::Bytes;
use std::sync::atomic::Ordering;

use super::state::{LEVEL_COUNT, Scope};

impl<FS: FileSystem> Scope<FS> {
    /// Collect the visible versions of one key at a read sequence.
    ///
    /// # Errors
    ///
    /// Returns an error when a storage read fails.
    pub fn get_versions(
        &self,
        user_key: &[u8],
        read_seq: SequenceNumber,
        now_ms: TimestampMs,
    ) -> Result<Vec<(ValueType, Bytes)>> {
        if self.is_dropped() {
            let drop_seq = self.drop_seq.load(Ordering::SeqCst);
            // Reads at or after the drop sequence observe an empty scope.
            // Snapshots taken strictly before the drop retain the old data.
            if read_seq.get() >= drop_seq {
                return Ok(Vec::new());
            }
        }

        let search_start = user_key_search_start(user_key);
        let mut versions = Vec::new();

        if collect_memtable_versions(
            self,
            &self.active_memtable.read(),
            user_key,
            &search_start,
            read_seq,
            now_ms,
            &mut versions,
        ) {
            return Ok(versions);
        }

        for memtable in self.flush_queue.all().iter().rev() {
            if collect_memtable_versions(
                self,
                memtable,
                user_key,
                &search_start,
                read_seq,
                now_ms,
                &mut versions,
            ) {
                return Ok(versions);
            }
        }

        let levels = self.sst_levels.read();
        for level in 0..LEVEL_COUNT {
            let Some(files) = levels.get(&level) else {
                continue;
            };
            for file_meta in files.iter().rev() {
                if collect_file_versions(self, file_meta, user_key, read_seq, now_ms, &mut versions)
                {
                    return Ok(versions);
                }
            }
        }

        Ok(versions)
    }

    /// Read the newest visible version of one key.
    ///
    /// # Errors
    ///
    /// Returns an error when a storage read fails.
    pub fn get(
        &self,
        user_key: &[u8],
        read_seq: SequenceNumber,
        now_ms: TimestampMs,
    ) -> Result<Option<(ValueType, Bytes)>> {
        let versions = self.get_versions(user_key, read_seq, now_ms)?;
        Ok(versions.into_iter().next())
    }
}

fn user_key_search_start(user_key: &[u8]) -> Bytes {
    InternalKey::new(
        Bytes::copy_from_slice(user_key),
        u64::MAX,
        ValueType::Value,
        0,
    )
    .encode()
}

fn collect_memtable_versions<FS: FileSystem>(
    scope: &Scope<FS>,
    memtable: &MemTable,
    user_key: &[u8],
    search_start: &Bytes,
    read_seq: SequenceNumber,
    now_ms: TimestampMs,
    versions: &mut Vec<(ValueType, Bytes)>,
) -> bool {
    for (encoded_key, value) in memtable.range_from(search_start.clone()) {
        let Some(internal_key) = InternalKey::decode(encoded_key) else {
            continue;
        };
        if internal_key.user_key.as_ref() != user_key {
            break;
        }
        if internal_key.seq > read_seq.get() {
            continue;
        }
        if is_expired(&internal_key, now_ms) {
            continue;
        }
        if hidden_by_range(scope, user_key, &internal_key, read_seq) {
            continue;
        }
        let is_terminal = internal_key.value_type != ValueType::MergeOperand;
        versions.push((internal_key.value_type, value));
        if is_terminal {
            return true;
        }
    }
    false
}

fn collect_file_versions<FS: FileSystem>(
    scope: &Scope<FS>,
    file_meta: &SstFileMeta,
    user_key: &[u8],
    read_seq: SequenceNumber,
    now_ms: TimestampMs,
    versions: &mut Vec<(ValueType, Bytes)>,
) -> bool {
    if !file_may_contain_key(file_meta, user_key) {
        return false;
    }
    if file_meta.max_expires_at_ms > 0 && file_meta.max_expires_at_ms <= now_ms.get() {
        return false;
    }

    let sst_path = scope
        .scope_dir
        .join(format!("{:016x}.sst", file_meta.file_num));
    let Ok(mut file) = scope.fs.open_file(&sst_path) else {
        return false;
    };
    let Ok(reader) = SstReader::open(&mut file) else {
        return false;
    };
    if !reader.filter.may_contain(user_key) {
        return false;
    }
    let Ok(entries) = reader.read_all_entries(&mut file) else {
        return false;
    };

    for (encoded_key, value) in entries {
        let Some(internal_key) = InternalKey::decode(encoded_key) else {
            continue;
        };
        if internal_key.user_key.as_ref() != user_key || internal_key.seq > read_seq.get() {
            continue;
        }
        if is_expired(&internal_key, now_ms) {
            continue;
        }
        if hidden_by_range(scope, user_key, &internal_key, read_seq) {
            continue;
        }
        let is_terminal = internal_key.value_type != ValueType::MergeOperand;
        versions.push((internal_key.value_type, value));
        if is_terminal {
            return true;
        }
    }
    false
}

fn hidden_by_range<FS: FileSystem>(
    scope: &Scope<FS>,
    user_key: &[u8],
    internal_key: &InternalKey,
    read_seq: SequenceNumber,
) -> bool {
    if internal_key.value_type == ValueType::RangeTombstone {
        // StorKV stores a range tombstone at its start key, but the record is
        // not a point write for that key. `is_range_deleted` decides whether
        // the tombstone hides older versions, so the record must not end the
        // version chain. This also keeps an empty range (`start == end`) a
        // no-op. The record does not delete the key at the start bound.
        return true;
    }
    scope.is_range_deleted(user_key, internal_key.seq, read_seq.get())
}

fn file_may_contain_key(file_meta: &SstFileMeta, user_key: &[u8]) -> bool {
    // `min_key`/`max_key` are encoded internal keys. Compare in user-key space
    // so a key equal to the boundary is not accidentally skipped (its internal
    // key is longer). Malformed metadata must not hide data; let the reader
    // validate the block and surface corruption.
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

fn is_expired(internal_key: &InternalKey, now_ms: TimestampMs) -> bool {
    internal_key.expires_at_ms != 0 && internal_key.expires_at_ms <= now_ms.get()
}
