//! Runtime storage statistics for observability endpoints.

use crate::db::DbInner;
use crate::vfs::FileSystem;
use std::sync::atomic::Ordering;

impl<FS: FileSystem> DbInner<FS> {
    /// Return the un-flushed WAL bytes and the total on-disk WAL bytes.
    pub(crate) fn wal_byte_stats(&self) -> crate::Result<(u64, u64)> {
        let pinned = self.unflushed_wal_bytes.load(Ordering::Relaxed);
        let mut total = 0u64;
        for path in self.fs.read_dir(&self.wal_dir)? {
            if path.extension().is_some_and(|extension| extension == "wal") {
                total = total.saturating_add(self.fs.file_size(&path)?);
            }
        }
        Ok((pinned, total))
    }
}
