//! Read-set validation for an optimistic commit.

use std::sync::atomic::Ordering;

use bytes::Bytes;

use crate::StorkvError;
use crate::db::DbInner;
use crate::format::{InternalKey, ValueType};
use crate::mvcc::TransactionContext;
use crate::scope::Scope;
use crate::vfs::FileSystem;

impl<FS: FileSystem + 'static> DbInner<FS> {
    pub(super) fn transaction_is_ephemeral(&self, transaction: &TransactionContext) -> bool {
        let scopes = self.scopes.read();
        transaction.write_set.keys().all(|(scope_id, _)| {
            scopes
                .get(scope_id)
                .is_some_and(|scope| scope.options.storage_mode == crate::StorageMode::InMemory)
        })
    }

    pub(super) fn validate_read_set(&self, transaction: &TransactionContext) -> crate::Result<()> {
        for (scope_id, user_key) in &transaction.read_set {
            let Some(scope) = self.scopes.read().get(scope_id).cloned() else {
                continue;
            };
            if read_set_was_invalidated(&scope, user_key, transaction.read_seq) {
                self.telemetry.record_write_conflict();
                return Err(StorkvError::WriteConflict {
                    scope_id: *scope_id,
                    key: user_key.to_vec(),
                });
            }
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
