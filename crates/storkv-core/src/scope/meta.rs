use crate::error::Result;
use crate::format::{InternalKey, ValueType};
use crate::memtable::MemTable;
use crate::sstable::SstReader;
use crate::types::{SequenceNumber, TimestampMs};
use crate::vfs::FileSystem;
use bytes::Bytes;
use std::sync::atomic::Ordering;

use super::state::{LEVEL_COUNT, Scope};

/// Metadata of the visible version of a key, used for exact TTL reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyMeta {
    /// Absolute Unix epoch milliseconds, `0` for a permanent key.
    pub expires_at_ms: u64,
    /// Sequence number of the visible version.
    pub seq: u64,
}

const _: () = assert!(std::mem::size_of::<KeyMeta>() == 16);

impl<FS: FileSystem> Scope<FS> {
    /// Look up the newest visible, non-expired version of `user_key` and return
    /// its expiry and sequence without materialising the value chain.
    pub fn key_meta(
        &self,
        user_key: &[u8],
        read_seq: SequenceNumber,
        now_ms: TimestampMs,
    ) -> Result<Option<KeyMeta>> {
        if self.is_dropped() {
            let drop_seq = self.drop_seq.load(Ordering::SeqCst);
            if read_seq.get() >= drop_seq {
                return Ok(None);
            }
        }

        let search_start = user_key_search_start(user_key);
        if let Some(meta) = meta_from_memtable(
            &self.active_memtable.read(),
            user_key,
            &search_start,
            read_seq,
            now_ms,
        ) {
            return Ok(meta);
        }
        for memtable in self.flush_queue.all().iter().rev() {
            if let Some(meta) =
                meta_from_memtable(memtable, user_key, &search_start, read_seq, now_ms)
            {
                return Ok(meta);
            }
        }

        let levels = self.sst_levels.read();
        for level in 0..LEVEL_COUNT {
            let Some(files) = levels.get(&level) else {
                continue;
            };
            for file_meta in files.iter().rev() {
                if let Some(meta) = meta_from_sst(self, file_meta, user_key, read_seq, now_ms) {
                    return Ok(meta);
                }
            }
        }
        Ok(None)
    }
}

fn meta_from_memtable(
    memtable: &MemTable,
    user_key: &[u8],
    search_start: &Bytes,
    read_seq: SequenceNumber,
    now_ms: TimestampMs,
) -> Option<Option<KeyMeta>> {
    for (encoded_key, _) in memtable.range_from(search_start.clone()) {
        let Some(internal_key) = InternalKey::decode(encoded_key) else {
            continue;
        };
        if internal_key.user_key.as_ref() != user_key {
            break;
        }
        if internal_key.seq > read_seq.get() || is_expired(&internal_key, now_ms) {
            continue;
        }
        if internal_key.value_type != ValueType::MergeOperand {
            return Some(visible_meta(&internal_key));
        }
    }
    None
}

fn meta_from_sst<FS: FileSystem>(
    scope: &Scope<FS>,
    file_meta: &crate::manifest::SstFileMeta,
    user_key: &[u8],
    read_seq: SequenceNumber,
    now_ms: TimestampMs,
) -> Option<Option<KeyMeta>> {
    if file_meta.max_expires_at_ms > 0 && file_meta.max_expires_at_ms <= now_ms.get() {
        return None;
    }
    let sst_path = scope
        .scope_dir
        .join(format!("{:016x}.sst", file_meta.file_num));
    let Ok(mut file) = scope.fs.open_file(&sst_path) else {
        return None;
    };
    let Ok(reader) = SstReader::open_cached(
        &mut file,
        &scope.block_cache,
        scope.scope_id,
        file_meta.file_num,
        true,
    ) else {
        return None;
    };
    if !reader.filter.may_contain(user_key) {
        return None;
    }
    let Ok(entries) = reader.read_all_entries_cached(
        &mut file,
        &scope.block_cache,
        scope.scope_id,
        file_meta.file_num,
        true,
    ) else {
        return None;
    };
    for (encoded_key, _) in entries {
        let Some(internal_key) = InternalKey::decode(encoded_key) else {
            continue;
        };
        if internal_key.user_key.as_ref() != user_key
            || internal_key.seq > read_seq.get()
            || is_expired(&internal_key, now_ms)
        {
            continue;
        }
        if internal_key.value_type != ValueType::MergeOperand {
            return Some(visible_meta(&internal_key));
        }
    }
    None
}

fn visible_meta(internal_key: &InternalKey) -> Option<KeyMeta> {
    match internal_key.value_type {
        ValueType::Value => Some(KeyMeta {
            expires_at_ms: internal_key.expires_at_ms,
            seq: internal_key.seq,
        }),
        _ => None,
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

fn is_expired(internal_key: &InternalKey, now_ms: TimestampMs) -> bool {
    internal_key.expires_at_ms != 0 && internal_key.expires_at_ms <= now_ms.get()
}
