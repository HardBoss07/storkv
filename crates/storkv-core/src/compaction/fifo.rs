use crate::error::Result;
use crate::manifest::{EditRecord, VersionEdit};
use crate::scope::Scope;
use crate::types::TimestampMs;
use crate::vfs::FileSystem;
use std::sync::Arc;

use super::Compactor;

impl Compactor {
    /// FIFO / Time-Window Compaction: Drop expired SSTables in O(1)
    pub(super) fn compact_fifo<FS: FileSystem>(
        scope: &Arc<Scope<FS>>,
        now_ms: TimestampMs,
    ) -> Result<Option<VersionEdit>> {
        let expired_files = collect_expired_files(scope, now_ms);
        if expired_files.is_empty() {
            return Ok(None);
        }

        let edit = delete_sst_edits(scope.scope_id, &expired_files);
        remove_expired_files(scope, &expired_files);
        Ok(Some(edit))
    }
}

fn collect_expired_files<FS: FileSystem>(
    scope: &Arc<Scope<FS>>,
    now_ms: TimestampMs,
) -> Vec<(u8, u64)> {
    let mut expired_files = Vec::new();
    let levels = scope.sst_levels.read();
    for (&level, files) in levels.iter() {
        for metadata in files {
            if metadata.max_expires_at_ms > 0 && metadata.max_expires_at_ms <= now_ms.get() {
                expired_files.push((level, metadata.file_num));
            }
        }
    }
    expired_files
}

fn delete_sst_edits(scope_id: u32, expired_files: &[(u8, u64)]) -> VersionEdit {
    let mut edit = VersionEdit::new();
    for (level, file_num) in expired_files {
        edit.add(EditRecord::DeleteSstFile {
            scope_id,
            level: *level,
            file_num: *file_num,
        });
    }
    edit
}

fn remove_expired_files<FS: FileSystem>(scope: &Arc<Scope<FS>>, expired_files: &[(u8, u64)]) {
    let mut levels = scope.sst_levels.write();
    for (level, file_num) in expired_files {
        if let Some(files) = levels.get_mut(level) {
            files.retain(|metadata| metadata.file_num != *file_num);
        }
        let path = scope.scope_dir.join(format!("{:016x}.sst", file_num));
        let _ = scope.fs.remove_file(&path);
    }
}
