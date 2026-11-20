use crate::db::DbInner;
use crate::format::{InternalKey, ValueType};
use crate::manifest::{EditRecord, SstFileMeta, VersionEdit};
use crate::memtable::MemTable;
use crate::scope::Scope;
use crate::sstable::{CompressionType, SstWriter};
use crate::vfs::FileSystem;
use std::sync::Arc;
use std::sync::atomic::Ordering;

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Rotate the active memtable, write it to an L0 SSTable, and record it.
    ///
    /// # Errors
    ///
    /// Returns an error when the SSTable cannot be written or logged.
    pub fn flush_scope_memtable(&self, scope: &Arc<Scope<FS>>) -> crate::Result<()> {
        if scope.options.storage_mode == crate::StorageMode::InMemory {
            return Ok(());
        }
        // Do not rotate an empty memtable. Rotation queues the memtable for
        // flush but writes nothing. A later non-empty flush writes the newer
        // memtable and pops the empty one. The written memtable then stays in
        // the queue, and the code counts its merge operands a second time.
        if scope.active_memtable.read().is_empty() {
            return Ok(());
        }
        let memtable = scope.rotate_memtable();

        let file_num = self.next_file_num.fetch_add(1, Ordering::SeqCst);
        let sst_meta = self.write_memtable_sstable(scope, &memtable, file_num)?;
        log_sst_file(self, sst_meta.clone())?;

        scope
            .sst_levels
            .write()
            .entry(0)
            .or_default()
            .push(sst_meta);
        let _ = scope.flush_queue.pop_oldest();

        Ok(())
    }

    fn write_memtable_sstable(
        &self,
        scope: &Arc<Scope<FS>>,
        memtable: &MemTable,
        file_num: u64,
    ) -> crate::Result<SstFileMeta> {
        let sst_path = scope.scope_dir.join(format!("{:016x}.sst", file_num));
        let mut file = scope.fs.create_file(&sst_path)?;

        let mut writer = SstWriter::new(
            &mut file,
            scope.scope_id,
            0,
            CompressionType::Lz4,
            scope.options.prefix_bloom_len,
        );
        write_memtable_entries(&mut writer, memtable)?;

        let (_footer, metadata, min_key, max_key) = writer.finish()?;
        Ok(SstFileMeta {
            scope_id: scope.scope_id,
            level: 0,
            file_num,
            file_size: metadata.raw_key_bytes + metadata.raw_val_bytes + 1024,
            min_seq: metadata.min_seq,
            max_seq: metadata.max_seq,
            max_expires_at_ms: metadata.max_expires_at_ms,
            min_key,
            max_key,
        })
    }
}

fn log_sst_file<FS: FileSystem>(db: &DbInner<FS>, sst_meta: SstFileMeta) -> crate::Result<()> {
    let mut edit = VersionEdit::new();
    let next_seq = db.last_visible_seq.load(Ordering::Acquire);
    edit.add(EditRecord::SetNextSequence(next_seq));
    edit.add(EditRecord::AddSstFile(sst_meta));
    db.manifest_mgr.log_edit(edit)
}

fn write_memtable_entries(writer: &mut SstWriter<'_>, memtable: &MemTable) -> crate::Result<()> {
    for (encoded_key, value) in memtable.iter() {
        if let Some(internal_key) = InternalKey::decode(encoded_key.clone()) {
            let is_tombstone = internal_key.value_type == ValueType::Tombstone;
            writer.add(
                &encoded_key,
                &internal_key.user_key,
                internal_key.seq,
                internal_key.expires_at_ms,
                is_tombstone,
                &value,
            )?;
        }
    }
    Ok(())
}
