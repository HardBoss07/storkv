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

use super::Compactor;

const LEVEL_ZERO: u8 = 0;
const LEVEL_ONE: u8 = 1;
const LEVEL_ZERO_TRIGGER: usize = 4;

impl Compactor {
    /// Leveled Compaction: Merge L0 files into L1
    pub(super) fn compact_leveled<FS: FileSystem>(
        scope: &Arc<Scope<FS>>,
        gc_safe_seq: SequenceNumber,
        next_file_num: &AtomicU64,
        now_ms: TimestampMs,
    ) -> Result<Option<VersionEdit>> {
        let Some(source_files) = select_source_files(scope) else {
            return Ok(None);
        };
        let source_entries = read_source_entries(scope, &source_files);
        if source_entries.is_empty() {
            return Ok(None);
        }

        let new_file_num = next_file_num.fetch_add(1, Ordering::SeqCst);
        let new_metadata =
            write_compacted_sstable(scope, new_file_num, source_entries, gc_safe_seq, now_ms)?;

        let mut edit = VersionEdit::new();
        for source in &source_files {
            edit.add(EditRecord::DeleteSstFile {
                scope_id: scope.scope_id,
                level: LEVEL_ZERO,
                file_num: source.file_num,
            });
        }
        edit.add(EditRecord::AddSstFile(new_metadata.clone()));

        apply_compaction_result(scope, &source_files, new_metadata);
        Ok(Some(edit))
    }
}

fn select_source_files<FS: FileSystem>(scope: &Arc<Scope<FS>>) -> Option<Vec<SstFileMeta>> {
    let levels = scope.sst_levels.read();
    let level_zero_files = levels.get(&LEVEL_ZERO).cloned().unwrap_or_default();
    if level_zero_files.len() >= LEVEL_ZERO_TRIGGER {
        Some(level_zero_files)
    } else {
        None
    }
}

fn read_source_entries<FS: FileSystem>(
    scope: &Arc<Scope<FS>>,
    source_files: &[SstFileMeta],
) -> BTreeMap<InternalKey, Bytes> {
    let mut all_entries = BTreeMap::new();
    for metadata in source_files {
        let sst_path = scope
            .scope_dir
            .join(format!("{:016x}.sst", metadata.file_num));
        let Ok(mut file) = scope.fs.open_file(&sst_path) else {
            continue;
        };
        let Ok(reader) = SstReader::open(&mut file) else {
            continue;
        };
        let Ok(entries) = reader.read_all_entries(&mut file) else {
            continue;
        };
        for (encoded_key, value) in entries {
            if let Some(internal_key) = InternalKey::decode(encoded_key) {
                all_entries.insert(internal_key, value);
            }
        }
    }
    all_entries
}

fn write_compacted_sstable<FS: FileSystem>(
    scope: &Arc<Scope<FS>>,
    new_file_num: u64,
    source_entries: BTreeMap<InternalKey, Bytes>,
    gc_safe_seq: SequenceNumber,
    now_ms: TimestampMs,
) -> Result<SstFileMeta> {
    let new_sst_path = scope.scope_dir.join(format!("{:016x}.sst", new_file_num));
    let mut new_file = scope.fs.create_file(&new_sst_path)?;
    let mut writer = SstWriter::new(
        &mut new_file,
        scope.scope_id,
        LEVEL_ONE,
        CompressionType::Lz4,
        scope.options.prefix_bloom_len,
    );

    let mut version_count_per_user_key: BTreeMap<Bytes, usize> = BTreeMap::new();
    for (internal_key, value) in source_entries {
        if is_expired(&internal_key, now_ms) {
            continue;
        }
        let version_count = version_count_per_user_key
            .entry(internal_key.user_key.clone())
            .or_insert(0);
        *version_count += 1;
        if *version_count > 1 && internal_key.seq < gc_safe_seq.get() {
            continue;
        }

        let encoded_key = internal_key.encode();
        writer.add(
            &encoded_key,
            &internal_key.user_key,
            internal_key.seq,
            internal_key.expires_at_ms,
            internal_key.value_type == ValueType::Tombstone,
            &value,
        )?;
    }

    let (_footer, metadata, min_key, max_key) = writer.finish()?;
    Ok(SstFileMeta {
        scope_id: scope.scope_id,
        level: LEVEL_ONE,
        file_num: new_file_num,
        file_size: metadata.raw_key_bytes + metadata.raw_val_bytes + 1024,
        min_seq: metadata.min_seq,
        max_seq: metadata.max_seq,
        max_expires_at_ms: metadata.max_expires_at_ms,
        min_key,
        max_key,
    })
}

fn apply_compaction_result<FS: FileSystem>(
    scope: &Arc<Scope<FS>>,
    source_files: &[SstFileMeta],
    new_metadata: SstFileMeta,
) {
    let mut levels = scope.sst_levels.write();
    if let Some(files) = levels.get_mut(&LEVEL_ZERO) {
        files.clear();
    }
    if let Some(files) = levels.get_mut(&LEVEL_ONE) {
        files.push(new_metadata);
    }
    for source in source_files {
        let path = scope
            .scope_dir
            .join(format!("{:016x}.sst", source.file_num));
        let _ = scope.fs.remove_file(&path);
    }
}

fn is_expired(internal_key: &InternalKey, now_ms: TimestampMs) -> bool {
    internal_key.expires_at_ms != 0 && internal_key.expires_at_ms <= now_ms.get()
}
