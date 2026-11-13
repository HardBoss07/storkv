use crate::error::Result;
use crate::format::{InternalKey, ValueType};
use crate::manifest::{EditRecord, SstFileMeta, VersionEdit};
use crate::scope::{CompactionStyle, Retention, Scope};
use crate::sstable::{CompressionType, SstReader, SstWriter};
use crate::vfs::FileSystem;
use bytes::Bytes;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Compactor;

impl Compactor {
    /// Run Leveled or FIFO compaction on a scope respecting gc_safe_seq
    pub fn compact_scope<FS: FileSystem>(
        scope: &Arc<Scope<FS>>,
        gc_safe_seq: u64,
        next_file_num: &AtomicU64,
        now_ms: u64,
    ) -> Result<Option<VersionEdit>> {
        if scope.is_dropped() {
            return Ok(None);
        }

        match scope.options.retention {
            Retention::Ttl {
                compaction_style: CompactionStyle::Fifo,
                ..
            } => Self::compact_fifo(scope, now_ms),
            _ => Self::compact_leveled(scope, gc_safe_seq, next_file_num, now_ms),
        }
    }

    /// FIFO / Time-Window Compaction: Drop expired SSTables in O(1)
    fn compact_fifo<FS: FileSystem>(
        scope: &Arc<Scope<FS>>,
        now_ms: u64,
    ) -> Result<Option<VersionEdit>> {
        let mut edit = VersionEdit::new();
        let mut files_to_drop = Vec::new();

        {
            let levels = scope.sst_levels.read();
            for (&level, files) in levels.iter() {
                for meta in files {
                    if meta.max_expires_at_ms > 0 && meta.max_expires_at_ms <= now_ms {
                        files_to_drop.push((level, meta.file_num));
                        edit.add(EditRecord::DeleteSstFile {
                            scope_id: scope.scope_id,
                            level,
                            file_num: meta.file_num,
                        });
                    }
                }
            }
        }

        if files_to_drop.is_empty() {
            return Ok(None);
        }

        {
            let mut levels = scope.sst_levels.write();
            for (level, file_num) in &files_to_drop {
                if let Some(list) = levels.get_mut(level) {
                    list.retain(|f| f.file_num != *file_num);
                }
                let path = scope.scope_dir.join(format!("{:016x}.sst", file_num));
                let _ = scope.fs.remove_file(&path);
            }
        }

        Ok(Some(edit))
    }

    /// Leveled Compaction: Merge L0 files into L1 or Ln into Ln+1
    fn compact_leveled<FS: FileSystem>(
        scope: &Arc<Scope<FS>>,
        gc_safe_seq: u64,
        next_file_num: &AtomicU64,
        now_ms: u64,
    ) -> Result<Option<VersionEdit>> {
        let (source_files, source_level, target_level) = {
            let levels = scope.sst_levels.read();
            let l0 = levels.get(&0).cloned().unwrap_or_default();
            if l0.len() >= 4 {
                (l0, 0u8, 1u8)
            } else {
                return Ok(None);
            }
        };

        let mut all_entries: BTreeMap<InternalKey, Bytes> = BTreeMap::new();
        for meta in &source_files {
            let sst_path = scope.scope_dir.join(format!("{:016x}.sst", meta.file_num));
            if let Ok(mut f) = scope.fs.open_file(&sst_path) {
                if let Ok(reader) = SstReader::open(&mut f) {
                    if let Ok(entries) = reader.read_all_entries(&mut f) {
                        for (enc_k, v) in entries {
                            if let Some(ikey) = InternalKey::decode(enc_k) {
                                all_entries.insert(ikey, v);
                            }
                        }
                    }
                }
            }
        }

        if all_entries.is_empty() {
            return Ok(None);
        }

        let new_file_num = next_file_num.fetch_add(1, Ordering::SeqCst);
        let new_sst_path = scope.scope_dir.join(format!("{:016x}.sst", new_file_num));
        let mut new_file = scope.fs.create_file(&new_sst_path)?;

        let mut writer = SstWriter::new(
            &mut new_file,
            scope.scope_id,
            target_level,
            CompressionType::Lz4,
            scope.options.prefix_bloom_len,
        );

        let mut version_count_per_user_key: BTreeMap<Bytes, usize> = BTreeMap::new();

        for (ikey, val) in all_entries {
            // Purge expired TTL entries
            if ikey.expires_at_ms != 0 && ikey.expires_at_ms <= now_ms {
                continue;
            }

            let count = version_count_per_user_key
                .entry(ikey.user_key.clone())
                .or_insert(0);
            *count += 1;

            // Garbage collect versions below gc_safe_seq if superseded
            if *count > 1 && ikey.seq < gc_safe_seq {
                continue;
            }

            let is_tombstone = ikey.value_type == ValueType::Tombstone;
            let enc_ikey = ikey.encode();
            writer.add(
                &enc_ikey,
                &ikey.user_key,
                ikey.seq,
                ikey.expires_at_ms,
                is_tombstone,
                &val,
            )?;
        }

        let (_footer, meta, min_key, max_key) = writer.finish()?;

        let mut edit = VersionEdit::new();
        // Remove source files
        for src in &source_files {
            edit.add(EditRecord::DeleteSstFile {
                scope_id: scope.scope_id,
                level: source_level,
                file_num: src.file_num,
            });
        }

        let new_meta = SstFileMeta {
            scope_id: scope.scope_id,
            level: target_level,
            file_num: new_file_num,
            file_size: meta.raw_key_bytes + meta.raw_val_bytes + 1024,
            min_seq: meta.min_seq,
            max_seq: meta.max_seq,
            max_expires_at_ms: meta.max_expires_at_ms,
            min_key,
            max_key,
        };
        edit.add(EditRecord::AddSstFile(new_meta.clone()));

        // Update scope in-memory state
        {
            let mut levels = scope.sst_levels.write();
            if let Some(list) = levels.get_mut(&source_level) {
                list.clear();
            }
            if let Some(list) = levels.get_mut(&target_level) {
                list.push(new_meta);
            }

            for src in &source_files {
                let path = scope.scope_dir.join(format!("{:016x}.sst", src.file_num));
                let _ = scope.fs.remove_file(&path);
            }
        }

        Ok(Some(edit))
    }
}
