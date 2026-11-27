//! Apply one compaction result to the manifest, cache, and file store.

use crate::db::DbInner;
use crate::manifest::{EditRecord, VersionEdit};
use crate::scope::Scope;
use crate::vfs::FileSystem;
use std::sync::Arc;

impl<FS: FileSystem + 'static> DbInner<FS> {
    pub(crate) fn apply_compaction_edit(
        &self,
        scope: &Arc<Scope<FS>>,
        edit: Option<VersionEdit>,
    ) -> crate::Result<bool> {
        let Some(edit) = edit else {
            return Ok(false);
        };
        let bytes_written: u64 = edit
            .edits
            .iter()
            .filter_map(|record| match record {
                EditRecord::AddSstFile(metadata) => Some(metadata.file_size),
                _ => None,
            })
            .sum();
        self.telemetry
            .record_compaction_bytes_written(bytes_written);
        // Persist the edit first. The manifest must never point at a file that
        // was already removed when the process restarts.
        let obsolete: Vec<u64> = edit
            .edits
            .iter()
            .filter_map(|record| match record {
                EditRecord::DeleteSstFile { file_num, .. } => Some(*file_num),
                _ => None,
            })
            .collect();
        self.manifest_mgr.log_edit(edit)?;
        for file_num in obsolete {
            self.block_cache.invalidate_sst(scope.scope_id, file_num);
            let path = scope.scope_dir.join(format!("{:016x}.sst", file_num));
            scope.fs.remove_file(&path)?;
        }
        self.notify_write_stall();
        Ok(true)
    }
}
