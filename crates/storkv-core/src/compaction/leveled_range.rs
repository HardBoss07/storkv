use crate::error::Result;
use crate::format::{InternalKey, ValueType};
use crate::manifest::{EditRecord, SstFileMeta, VersionEdit};
use crate::scope::Scope;
use crate::sstable::{CompressionType, SstReader, SstWriter};
use crate::types::{SequenceNumber, TimestampMs};
use crate::vfs::FileSystem;
use bytes::Bytes;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::range_filter::{filter_entries, select_overlapping};
use super::{Compactor, RateLimiter};

/// Lowest level that receives a compacted range. L0 files merge into L1.
const TARGET_MIN_LEVEL: u8 = 1;

impl Compactor {
    /// Merge the SSTables that overlap `[start, end)` into the highest
    /// selected level. Files outside the range stay untouched.
    pub(crate) fn compact_range<FS: FileSystem>(
        scope: &Arc<Scope<FS>>,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
        gc_safe_seq: SequenceNumber,
        next_file_num: &AtomicU64,
        now_ms: TimestampMs,
        rate_limiter: Option<&RateLimiter>,
    ) -> Result<Option<VersionEdit>> {
        if scope.is_dropped() {
            return Ok(None);
        }
        let sources = select_overlapping(scope, start, end);
        if sources.is_empty() {
            return Ok(None);
        }

        let target_level = sources
            .iter()
            .map(|(level, _)| *level)
            .max()
            .unwrap_or(TARGET_MIN_LEVEL)
            .max(TARGET_MIN_LEVEL);
        let entries = read_source_entries(scope, &sources)?;
        let entries = filter_entries(scope, entries, start, end, gc_safe_seq, now_ms);

        let new_metadata = if entries.is_empty() {
            None
        } else {
            let new_file_num = next_file_num.fetch_add(1, Ordering::SeqCst);
            Some(write_range_sstable(
                scope,
                new_file_num,
                target_level,
                entries,
                rate_limiter,
            )?)
        };

        let mut edit = VersionEdit::new();
        for (level, source) in &sources {
            edit.add(EditRecord::DeleteSstFile {
                scope_id: scope.scope_id,
                level: *level,
                file_num: source.file_num,
            });
        }
        if let Some(metadata) = &new_metadata {
            edit.add(EditRecord::AddSstFile(metadata.clone()));
        }

        apply_range_result(scope, &sources, new_metadata);
        Ok(Some(edit))
    }
}

fn read_source_entries<FS: FileSystem>(
    scope: &Arc<Scope<FS>>,
    sources: &[(u8, SstFileMeta)],
) -> Result<BTreeMap<InternalKey, Bytes>> {
    let mut all_entries = BTreeMap::new();
    for (_, metadata) in sources {
        let sst_path = scope
            .scope_dir
            .join(format!("{:016x}.sst", metadata.file_num));
        let mut file = scope.fs.open_file(&sst_path)?;
        let reader = SstReader::open_cached(
            &mut file,
            &scope.block_cache,
            scope.scope_id,
            metadata.file_num,
            false,
        )?;
        for (encoded_key, value) in reader.read_all_entries_cached(
            &mut file,
            &scope.block_cache,
            scope.scope_id,
            metadata.file_num,
            false,
        )? {
            if let Some(internal_key) = InternalKey::decode(encoded_key) {
                all_entries.insert(internal_key, value);
            }
        }
    }
    Ok(all_entries)
}

fn write_range_sstable<FS: FileSystem>(
    scope: &Arc<Scope<FS>>,
    new_file_num: u64,
    target_level: u8,
    entries: Vec<(InternalKey, Bytes)>,
    rate_limiter: Option<&RateLimiter>,
) -> Result<SstFileMeta> {
    let new_sst_path = scope.scope_dir.join(format!("{:016x}.sst", new_file_num));
    let mut new_file = scope.fs.create_file(&new_sst_path)?;
    let mut writer = SstWriter::new(
        &mut new_file,
        scope.scope_id,
        target_level,
        CompressionType::Lz4,
        scope.options.prefix_bloom_len,
    );

    let mut throttled_bytes: u64 = 0;
    for (internal_key, value) in entries {
        let encoded_key = internal_key.encode();
        writer.add(
            &encoded_key,
            &internal_key.user_key,
            internal_key.seq,
            internal_key.expires_at_ms,
            internal_key.value_type == ValueType::Tombstone,
            &value,
        )?;
        if let Some(limiter) = rate_limiter {
            let written = writer.bytes_written();
            if written > throttled_bytes {
                limiter.throttle(usize::try_from(written - throttled_bytes).unwrap_or(usize::MAX));
                throttled_bytes = written;
            }
        }
    }

    let (_footer, metadata, min_key, max_key) = writer.finish()?;
    let file_size = new_file.file_size()?;
    if let Some(limiter) = rate_limiter {
        if file_size > throttled_bytes {
            limiter.throttle(usize::try_from(file_size - throttled_bytes).unwrap_or(usize::MAX));
        }
    }
    Ok(SstFileMeta {
        scope_id: scope.scope_id,
        level: target_level,
        file_num: new_file_num,
        file_size,
        min_seq: metadata.min_seq,
        max_seq: metadata.max_seq,
        max_expires_at_ms: metadata.max_expires_at_ms,
        min_key,
        max_key,
    })
}

fn apply_range_result<FS: FileSystem>(
    scope: &Arc<Scope<FS>>,
    sources: &[(u8, SstFileMeta)],
    new_metadata: Option<SstFileMeta>,
) {
    let mut levels = scope.sst_levels.write();
    for (level, source) in sources {
        if let Some(files) = levels.get_mut(level) {
            files.retain(|file| file.file_num != source.file_num);
        }
    }
    if let Some(metadata) = new_metadata {
        levels.entry(metadata.level).or_default().push(metadata);
    }
}
