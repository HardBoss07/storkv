//! Transaction commit orchestration.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::db::DbInner;
use crate::mvcc::{TransactionContext, TxMutation};
use crate::scope::Scope;
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
        self.commit_tx_with_options(transaction, crate::WriteOptions::default())
    }

    /// Validate the read set, append the write set to the WAL, and apply it
    /// with explicit write options.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorkvError::WriteConflict`] when a read key changed
    /// after the transaction started,
    /// [`crate::StorkvError::WriteStalled`] in non-blocking mode when a
    /// backpressure watermark is exceeded, or an I/O error.
    pub fn commit_tx_with_options(
        &self,
        transaction: TransactionContext,
        options: crate::WriteOptions,
    ) -> crate::Result<()> {
        if !transaction.write_set.is_empty() {
            let scope_ids: Vec<u32> = transaction
                .write_set
                .keys()
                .map(|(scope_id, _)| *scope_id)
                .collect();
            self.wait_for_write_capacity(&scope_ids, options)?;
        }
        let touched_scopes = {
            let _commit_guard = self.commit_lock.lock();
            self.commit_tx_locked(transaction)?
        };
        for scope in touched_scopes {
            self.maybe_flush_memtable(&scope)?;
        }
        Ok(())
    }

    /// Commit while the caller already holds the commit lock.
    pub(crate) fn commit_tx_locked(
        &self,
        transaction: TransactionContext,
    ) -> crate::Result<Vec<Arc<Scope<FS>>>> {
        self.validate_read_set(&transaction)?;

        if transaction.write_set.is_empty() {
            return Ok(Vec::new());
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
        Ok(touched)
    }
}
