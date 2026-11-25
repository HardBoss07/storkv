//! SSTable collection helpers for database scans.

use std::sync::Arc;

use super::scan::ScanState;
use crate::db::DbInner;
use crate::scope::Scope;
use crate::sstable::SstReader;
use crate::vfs::FileSystem;

impl<FS: FileSystem + 'static> DbInner<FS> {
    pub(super) fn collect_sstables(&self, scope: &Arc<Scope<FS>>, state: &mut ScanState) {
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
        let Ok(reader) = SstReader::open_cached(
            &mut file,
            &self.block_cache,
            scope.scope_id,
            file_num,
            false,
        ) else {
            return;
        };
        let Ok(entries) = reader.read_all_entries_cached(
            &mut file,
            &self.block_cache,
            scope.scope_id,
            file_num,
            false,
        ) else {
            return;
        };
        for (encoded_key, value) in entries {
            state.add_entry(encoded_key, value);
        }
    }
}
