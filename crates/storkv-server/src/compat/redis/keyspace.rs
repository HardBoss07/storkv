//! Keyspace helpers shared by the Redis and Memcached adapters.
//!
//! Read-modify-write commands run inside a single OCC transaction so counters,
//! CAS, and conditional writes stay atomic under concurrent connections.

use std::time::Duration;

use bytes::Bytes;
use storkv_core::mvcc::{TransactionContext, TxMutation};
use storkv_core::{Db, ScopeHandle, SequenceNumber, StorkvError, TimestampMs};

const MAX_RETRIES: usize = 16;

pub(crate) enum Outcome<T> {
    Commit(T),
    Reply(T),
}

pub(crate) struct Atomic<'a> {
    db: &'a Db,
    read_seq: u64,
    context: TransactionContext,
}

impl<'a> Atomic<'a> {
    pub(crate) fn get(
        &mut self,
        scope: &ScopeHandle,
        key: &[u8],
    ) -> storkv_core::Result<Option<Bytes>> {
        self.context
            .record_read(scope.scope_id, Bytes::copy_from_slice(key));
        if let Some(mutation) = self.context.get_local(scope.scope_id, key) {
            return Ok(match mutation {
                TxMutation::Put { val, .. } => Some(val.clone()),
                TxMutation::Delete | TxMutation::DeleteRange { .. } => None,
                TxMutation::Merge { .. } => self.db.inner().get_at(
                    scope.scope_id,
                    key,
                    SequenceNumber::new(self.read_seq),
                )?,
            });
        }
        self.db
            .inner()
            .get_at(scope.scope_id, key, SequenceNumber::new(self.read_seq))
    }

    pub(crate) fn put(
        &mut self,
        scope: &ScopeHandle,
        key: &[u8],
        value: &[u8],
        expires_at_ms: u64,
    ) {
        self.context.put(
            scope.scope_id,
            Bytes::copy_from_slice(key),
            Bytes::copy_from_slice(value),
            expires_at_ms,
        );
    }

    pub(crate) fn delete(&mut self, scope: &ScopeHandle, key: &[u8]) {
        self.context
            .delete(scope.scope_id, Bytes::copy_from_slice(key));
    }

    /// Absolute expiry of a live key, including buffered writes in this atomic.
    pub(crate) fn expires_at(
        &mut self,
        scope: &ScopeHandle,
        key: &[u8],
    ) -> storkv_core::Result<Option<u64>> {
        self.context
            .record_read(scope.scope_id, Bytes::copy_from_slice(key));
        if let Some(mutation) = self.context.get_local(scope.scope_id, key) {
            return Ok(match mutation {
                TxMutation::Put { expires_at_ms, .. } => Some(*expires_at_ms),
                TxMutation::Delete | TxMutation::DeleteRange { .. } => None,
                TxMutation::Merge { .. } => {
                    self.db.key_meta(scope, key)?.map(|meta| meta.expires_at_ms)
                }
            });
        }
        Ok(self.db.key_meta(scope, key)?.map(|meta| meta.expires_at_ms))
    }
}

pub(crate) fn atomic<T, F>(db: &Db, primary: bool, mut operation: F) -> storkv_core::Result<T>
where
    F: FnMut(&mut Atomic<'_>) -> storkv_core::Result<Outcome<T>>,
{
    if !primary {
        return Err(StorkvError::NotPrimary);
    }
    for _ in 0..MAX_RETRIES {
        let read_seq = db
            .inner()
            .last_visible_seq
            .load(std::sync::atomic::Ordering::Acquire);
        let mut atomic = Atomic {
            db,
            read_seq,
            context: TransactionContext::new(read_seq),
        };
        match operation(&mut atomic)? {
            Outcome::Reply(value) => return Ok(value),
            Outcome::Commit(value) => match db.inner().commit_tx(atomic.context) {
                Ok(()) => return Ok(value),
                Err(StorkvError::WriteConflict { .. }) => continue,
                Err(error) => return Err(error),
            },
        }
    }
    Err(StorkvError::WriteConflict {
        scope_id: 0,
        key: Vec::new(),
    })
}

pub(crate) fn put_absolute(
    db: &Db,
    primary: bool,
    scope: &ScopeHandle,
    key: &[u8],
    value: &[u8],
    expires_at_ms: u64,
) -> storkv_core::Result<()> {
    atomic(db, primary, |atomic| {
        atomic.put(scope, key, value, expires_at_ms);
        Ok(Outcome::Commit(()))
    })
}

/// Absolute expiry from a relative duration; `None` means permanent.
pub(crate) fn expiry_from_ttl(ttl: Option<Duration>) -> u64 {
    match ttl {
        Some(ttl) => TimestampMs::now()
            .get()
            .saturating_add(u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX)),
        None => 0,
    }
}

pub(crate) fn remaining_ms(
    db: &Db,
    scope: &ScopeHandle,
    key: &[u8],
) -> storkv_core::Result<Option<u64>> {
    db.key_ttl_remaining_ms(scope, key)
}
