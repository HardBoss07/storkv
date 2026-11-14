use crate::StorKvError;
use crate::db::DbInner;
use crate::format::{InternalKey, ValueType};
use crate::mvcc::{TransactionContext, TxMutation};
use crate::scope::Scope;
use crate::vfs::FileSystem;
use crate::wal::{Durability, WalBatchFrame, WalOp};
use crate::watch::ChangeEvent;
use bytes::Bytes;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

impl<FS: FileSystem + 'static> DbInner<FS> {
    pub fn commit_tx(&self, transaction: TransactionContext) -> crate::Result<()> {
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

        self.wal_mgr.write_batch(&frame, durability)?;
        if durability == Durability::StrictFsync {
            self.telemetry.record_fsync();
        }

        self.apply_write_set(transaction.write_set, &frame, batch_seq)
    }

    fn validate_read_set(&self, transaction: &TransactionContext) -> crate::Result<()> {
        for (scope_id, user_key) in &transaction.read_set {
            let Some(scope) = self.scopes.read().get(scope_id).cloned() else {
                continue;
            };
            if read_set_was_invalidated(&scope, user_key, transaction.read_seq) {
                self.telemetry.record_write_conflict();
                return Err(StorKvError::WriteConflict {
                    scope_id: *scope_id,
                    key: user_key.to_vec(),
                });
            }
        }
        Ok(())
    }

    fn apply_write_set(
        &self,
        write_set: BTreeMap<(u32, Bytes), TxMutation>,
        frame: &WalBatchFrame,
        batch_seq: u64,
    ) -> crate::Result<()> {
        for (index, (((scope_id, user_key), mutation), _op)) in
            write_set.into_iter().zip(&frame.ops).enumerate()
        {
            let op_seq = batch_seq + index as u64;
            self.telemetry.record_write();

            if let Some(scope) = self.scopes.read().get(&scope_id) {
                apply_mutation(self, scope, user_key, mutation, op_seq);
                self.maybe_flush_memtable(scope)?;
            }
        }
        Ok(())
    }

    fn maybe_flush_memtable(&self, scope: &Arc<Scope<FS>>) -> crate::Result<()> {
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

fn read_set_was_invalidated<FS: FileSystem>(
    scope: &Scope<FS>,
    user_key: &Bytes,
    read_seq: u64,
) -> bool {
    if scope.is_dropped() && scope.drop_seq.load(Ordering::SeqCst) > read_seq {
        return true;
    }
    has_newer_version(scope, user_key, read_seq)
}

fn build_wal_ops<FS: FileSystem + 'static>(
    db: &DbInner<FS>,
    transaction: &TransactionContext,
) -> (Vec<WalOp>, Durability) {
    let mut ops = Vec::with_capacity(transaction.write_set.len());
    let mut durability = Durability::Buffered {
        sync_interval: Duration::from_millis(100),
    };

    for ((scope_id, user_key), mutation) in &transaction.write_set {
        if let Some(scope) = db.scopes.read().get(scope_id) {
            if scope.options.durability == Durability::StrictFsync {
                durability = Durability::StrictFsync;
            }
        }
        ops.push(wal_op_for_mutation(*scope_id, user_key, mutation));
    }

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

fn has_newer_version<FS: FileSystem>(scope: &Scope<FS>, user_key: &Bytes, read_seq: u64) -> bool {
    let search_start = InternalKey::new(user_key.clone(), u64::MAX, ValueType::Value, 0).encode();
    for (entry_key, _) in scope.active_memtable.read().range_from(search_start) {
        let Some(internal_key) = InternalKey::decode(entry_key) else {
            break;
        };
        if internal_key.user_key != *user_key {
            break;
        }
        if internal_key.seq > read_seq {
            return true;
        }
    }
    false
}

fn apply_mutation<FS: FileSystem>(
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
            scope
                .active_memtable
                .read()
                .insert(internal_key.encode(), val.clone(), op_seq);
            db.watch_registry.broadcast(ChangeEvent::Put {
                scope_id: scope.scope_id,
                key: user_key,
                value: val,
                seq: op_seq,
            });
        }
        TxMutation::Delete => {
            let internal_key = InternalKey::new(user_key.clone(), op_seq, ValueType::Tombstone, 0);
            scope
                .active_memtable
                .read()
                .insert(internal_key.encode(), Bytes::new(), op_seq);
            db.watch_registry.broadcast(ChangeEvent::Delete {
                scope_id: scope.scope_id,
                key: user_key,
                seq: op_seq,
            });
        }
        TxMutation::Merge { operand } => {
            let internal_key = InternalKey::new(user_key, op_seq, ValueType::MergeOperand, 0);
            scope
                .active_memtable
                .read()
                .insert(internal_key.encode(), operand, op_seq);
        }
        TxMutation::DeleteRange { end_key } => {
            let internal_key = InternalKey::new(user_key, op_seq, ValueType::RangeTombstone, 0);
            scope
                .active_memtable
                .read()
                .insert(internal_key.encode(), end_key, op_seq);
        }
    }
}
