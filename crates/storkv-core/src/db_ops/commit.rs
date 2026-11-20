//! Transaction commit orchestration.

use std::sync::atomic::Ordering;

use crate::db::DbInner;
use crate::mvcc::{TransactionContext, TxMutation};
use crate::vfs::FileSystem;
use crate::wal::{Durability, WalBatchFrame};

mod apply;
mod validate;

use apply::build_wal_ops;

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Validate the read set, append the write set to the WAL, and apply it.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorkvError::WriteConflict`] when a read key changed after the
    /// transaction started, or an I/O error.
    pub fn commit_tx(&self, transaction: TransactionContext) -> crate::Result<()> {
        let touched_scopes = {
            let _commit_guard = self.commit_lock.lock();
            self.validate_read_set(&transaction)?;

            if transaction.write_set.is_empty() {
                return Ok(());
            }

            let entry_count = transaction.write_set.len() as u64;
            let batch_seq = self
                .last_visible_seq
                .fetch_add(entry_count, Ordering::SeqCst)
                + 1;
            let (ops, durability) = build_wal_ops(self, &transaction);
            let frame = WalBatchFrame::new(batch_seq, ops);

            if !self.transaction_is_ephemeral(&transaction) {
                self.wal_mgr.write_batch(&frame, durability)?;
                if durability == Durability::StrictFsync {
                    self.telemetry.record_fsync();
                }
            }

            let tombstones = transaction.write_set.iter().enumerate().filter_map(
                |(index, ((scope_id, start_key), mutation))| match mutation {
                    TxMutation::DeleteRange { end_key } => Some((
                        *scope_id,
                        batch_seq + index as u64,
                        start_key.clone(),
                        end_key.clone(),
                    )),
                    _ => None,
                },
            );
            self.log_range_tombstones(tombstones)?;

            let touched = self.apply_write_set(transaction.write_set, batch_seq)?;
            self.replication_registry.broadcast(frame);
            touched
        };

        // Flushing writes an SSTable and updates the manifest. It is slow and
        // does not need the commit lock: the committed memtable is already
        // visible through the flush queue, and the per-scope flush lock
        // serializes rotation with maintenance.
        for scope in touched_scopes {
            self.maybe_flush_memtable(&scope)?;
        }
        Ok(())
    }
}
