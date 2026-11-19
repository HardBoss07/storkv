//! Optimistic transaction over a database snapshot.

use std::sync::Arc;

use crate::mvcc::TransactionContext;
use crate::vfs::{AnyFs, FileSystem, PosixFs};

mod read;
mod write;

/// Optimistic transaction over a [`crate::Db`].
///
/// The transaction reads at a fixed sequence number, buffers writes, and
/// validates its read set on commit. Reads see buffered writes first.
pub struct Transaction<FS: FileSystem + 'static = AnyFs<PosixFs>> {
    pub(crate) db: Arc<crate::db::DbInner<FS>>,
    pub(crate) ctx: TransactionContext,
}

impl<FS: FileSystem + 'static> Drop for Transaction<FS> {
    fn drop(&mut self) {
        let read_seq = self.ctx.read_seq;
        self.release_reader(read_seq);
    }
}
