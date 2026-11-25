//! WAL encoding and memtable application for a commit.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;

use crate::db::DbInner;
use crate::format::{InternalKey, ValueType};
use crate::manifest::{EditRecord, VersionEdit};
use crate::mvcc::{TransactionContext, TxMutation};
use crate::scope::Scope;
use crate::vfs::FileSystem;
use crate::wal::{Durability, WalOp};
use crate::watch::ChangeEvent;

const DEFAULT_BUFFERED_SYNC_INTERVAL_MS: u64 = 100;

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Persist range tombstones before the memtable applies them.
    ///
    /// The manifest index makes point reads correct after a restart. A repeated
    /// tombstone is harmless because the index deduplicates it.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest append fails.
    pub(in crate::db_ops) fn log_range_tombstones<I>(&self, tombstones: I) -> crate::Result<()>
    where
        I: IntoIterator<Item = (u32, u64, Bytes, Bytes)>,
    {
        let mut edit = VersionEdit::new();
        for (scope_id, seq, start_key, end_key) in tombstones {
            edit.add(EditRecord::AddRangeTombstone {
                scope_id,
                seq,
                start_key,
                end_key,
            });
        }
        if edit.edits.is_empty() {
            return Ok(());
        }
        self.manifest_mgr.log_edit(edit)
    }

    pub(super) fn apply_write_set(
        &self,
        write_set: BTreeMap<(u32, Bytes), TxMutation>,
        batch_seq: u64,
    ) -> crate::Result<Vec<Arc<Scope<FS>>>> {
        let scopes = self.scopes.read();
        let mut touched = Vec::new();
        let mut last_scope_id = None;

        for (index, ((scope_id, user_key), mutation)) in write_set.into_iter().enumerate() {
            let op_seq = batch_seq + index as u64;
            self.telemetry.record_write();

            if let Some(scope) = scopes.get(&scope_id).cloned() {
                apply_mutation(self, &scope, user_key, mutation, op_seq);
                if last_scope_id != Some(scope_id) {
                    last_scope_id = Some(scope_id);
                    touched.push(scope);
                }
            }
        }
        Ok(touched)
    }

    pub(in crate::db_ops) fn maybe_flush_memtable(
        &self,
        scope: &Arc<Scope<FS>>,
    ) -> crate::Result<()> {
        if scope.options.storage_mode == crate::StorageMode::InMemory {
            return Ok(());
        }
        let memtable = scope.active_memtable.read();
        let should_flush = memtable.size_bytes() >= self.options.max_wal_pin_bytes
            || memtable.created_at().elapsed() >= self.options.max_wal_pin_age;
        drop(memtable);
        if should_flush {
            self.flush_scope_memtable(scope)?;
        }
        Ok(())
    }
}

pub(super) fn build_wal_ops<FS: FileSystem + 'static>(
    db: &DbInner<FS>,
    transaction: &TransactionContext,
) -> (Vec<WalOp>, Durability) {
    let mut ops = Vec::with_capacity(transaction.write_set.len());
    let mut durability = Durability::Buffered {
        sync_interval: Duration::from_millis(DEFAULT_BUFFERED_SYNC_INTERVAL_MS),
    };

    let scopes = db.scopes.read();
    for ((scope_id, user_key), mutation) in &transaction.write_set {
        if let Some(scope) = scopes.get(scope_id) {
            if scope.options.durability == Durability::StrictFsync {
                durability = Durability::StrictFsync;
            }
        }
        ops.push(wal_op_for_mutation(*scope_id, user_key, mutation));
    }
    drop(scopes);

    (ops, durability)
}

fn wal_op_for_mutation(scope_id: u32, user_key: &Bytes, mutation: &TxMutation) -> WalOp {
    match mutation {
        TxMutation::Put { val, expires_at_ms } => WalOp::Put {
            scope_id,
            expires_at_ms: *expires_at_ms,
            key: user_key.clone(),
            val: val.clone(),
        },
        TxMutation::Delete => WalOp::Delete {
            scope_id,
            key: user_key.clone(),
        },
        TxMutation::Merge { operand } => WalOp::Merge {
            scope_id,
            key: user_key.clone(),
            val: operand.clone(),
        },
        TxMutation::DeleteRange { end_key } => WalOp::DeleteRange {
            scope_id,
            start_key: user_key.clone(),
            end_key: end_key.clone(),
        },
    }
}

fn apply_mutation<FS: FileSystem + 'static>(
    db: &DbInner<FS>,
    scope: &Scope<FS>,
    user_key: Bytes,
    mutation: TxMutation,
    op_seq: u64,
) {
    match mutation {
        TxMutation::Put { val, expires_at_ms } => {
            let internal_key =
                InternalKey::new(user_key.clone(), op_seq, ValueType::Value, expires_at_ms);
            let added =
                scope
                    .active_memtable
                    .read()
                    .insert(internal_key.encode(), val.clone(), op_seq);
            db.record_unflushed_wal_bytes(scope, added);
            if scope.options.active_ttl_tracking && expires_at_ms > 0 {
                scope
                    .ttl_tracker
                    .lock()
                    .push(std::cmp::Reverse((expires_at_ms, user_key.clone())));
            }
            db.watch_registry.broadcast(ChangeEvent::Put {
                scope_id: scope.scope_id,
                key: user_key,
                value: val,
                seq: op_seq,
                expires_at_ms,
            });
        }
        TxMutation::Delete => {
            let internal_key = InternalKey::new(user_key.clone(), op_seq, ValueType::Tombstone, 0);
            let added =
                scope
                    .active_memtable
                    .read()
                    .insert(internal_key.encode(), Bytes::new(), op_seq);
            db.record_unflushed_wal_bytes(scope, added);
            db.watch_registry.broadcast(ChangeEvent::Delete {
                scope_id: scope.scope_id,
                key: user_key,
                seq: op_seq,
            });
        }
        TxMutation::Merge { operand } => {
            let internal_key = InternalKey::new(user_key, op_seq, ValueType::MergeOperand, 0);
            let added = scope
                .active_memtable
                .read()
                .insert(internal_key.encode(), operand, op_seq);
            db.record_unflushed_wal_bytes(scope, added);
            db.watch_registry.broadcast(ChangeEvent::InvalidateScope {
                scope_id: scope.scope_id,
                seq: op_seq,
            });
        }
        TxMutation::DeleteRange { end_key } => {
            scope.record_range_tombstone(user_key.clone(), end_key.clone(), op_seq);
            let internal_key = InternalKey::new(user_key, op_seq, ValueType::RangeTombstone, 0);
            let added = scope
                .active_memtable
                .read()
                .insert(internal_key.encode(), end_key, op_seq);
            db.record_unflushed_wal_bytes(scope, added);
            db.watch_registry.broadcast(ChangeEvent::InvalidateScope {
                scope_id: scope.scope_id,
                seq: op_seq,
            });
        }
    }
}
