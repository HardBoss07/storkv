use crate::db::DbInner;
use crate::vfs::FileSystem;
use std::path::Path;

impl<FS: FileSystem + 'static> DbInner<FS> {
    pub fn checkpoint(&self, dest_dir: &Path) -> crate::Result<()> {
        self.fs.create_dir_all(dest_dir)?;
        self.flush_all_scopes()?;
        self.link_live_sstables(dest_dir)?;
        self.copy_manifest_files(dest_dir)?;
        self.copy_wal_segments(dest_dir)?;
        self.fs.sync_dir(dest_dir)?;
        Ok(())
    }

    fn flush_all_scopes(&self) -> crate::Result<()> {
        let scopes: Vec<_> = self.scopes.read().values().cloned().collect();
        for scope in &scopes {
            self.flush_scope_memtable(scope)?;
        }
        Ok(())
    }

    fn link_live_sstables(&self, dest_dir: &Path) -> crate::Result<()> {
        let dest_scopes = dest_dir.join("scopes");
        self.fs.create_dir_all(&dest_scopes)?;

        let state = self.manifest_mgr.state();
        for (scope_id, _level, file_num) in state.sst_files.keys() {
            let scope_subdir = dest_scopes.join(format!("{:08x}", scope_id));
            self.fs.create_dir_all(&scope_subdir)?;

            let file_name = format!("{:016x}.sst", file_num);
            let source = self
                .scopes_dir
                .join(format!("{:08x}", scope_id))
                .join(&file_name);
            let destination = scope_subdir.join(&file_name);
            if self.fs.exists(&source) {
                let _ = self.fs.hard_link(&source, &destination);
            }
        }
        Ok(())
    }

    fn copy_manifest_files(&self, dest_dir: &Path) -> crate::Result<()> {
        let current_source = self.db_root.join("CURRENT");
        let current_destination = dest_dir.join("CURRENT");
        let Ok(mut current_file) = self.fs.open_file(&current_source) else {
            return Ok(());
        };

        let mut current_contents = Vec::new();
        current_file.read_to_end(&mut current_contents)?;
        let mut destination_file = self.fs.create_file(&current_destination)?;
        destination_file.write_all(&current_contents)?;
        destination_file.sync_all()?;

        let manifest_name = String::from_utf8_lossy(&current_contents)
            .trim()
            .to_string();
        let manifest_source = self.db_root.join(&manifest_name);
        let manifest_destination = dest_dir.join(&manifest_name);
        let Ok(mut manifest_file) = self.fs.open_file(&manifest_source) else {
            return Ok(());
        };

        let mut manifest_contents = Vec::new();
        manifest_file.read_to_end(&mut manifest_contents)?;
        let mut destination_file = self.fs.create_file(&manifest_destination)?;
        destination_file.write_all(&manifest_contents)?;
        destination_file.sync_all()?;
        Ok(())
    }

    fn copy_wal_segments(&self, dest_dir: &Path) -> crate::Result<()> {
        let destination_wal = dest_dir.join("wal");
        self.fs.create_dir_all(&destination_wal)?;

        for source_path in self.fs.read_dir(&self.wal_dir).unwrap_or_default() {
            let Some(file_name) = source_path.file_name() else {
                continue;
            };
            let destination_path = destination_wal.join(file_name);
            let Ok(mut source_file) = self.fs.open_file(&source_path) else {
                continue;
            };

            let mut contents = Vec::new();
            source_file.read_to_end(&mut contents)?;
            let mut destination_file = self.fs.create_file(&destination_path)?;
            destination_file.write_all(&contents)?;
            destination_file.sync_all()?;
        }
        Ok(())
    }
}
