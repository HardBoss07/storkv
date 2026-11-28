//! Atomic single-key conditional writes.

use std::sync::atomic::Ordering;

use bytes::Bytes;

use crate::db::DbInner;
use crate::mvcc::{PutCondition, PutIfResult, TransactionContext, TxMutation};
use crate::types::{SequenceNumber, TimestampMs};
use crate::vfs::FileSystem;

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Evaluate `condition` and apply `mutation` under the commit lock.
    ///
    /// The condition reads the newest visible state, so no OCC read set and no
    /// retry loop are needed.
    ///
    /// # Errors
    ///
    /// Returns `crate::StorkvError::ScopeIdNotFound` when the scope is
    /// unknown, or a storage or WAL error.
    pub fn put_if(
        &self,
        scope_id: u32,
        user_key: Bytes,
        condition: &PutCondition,
        mutation: TxMutation,
    ) -> crate::Result<PutIfResult> {
        self.put_if_with_options(
            scope_id,
            user_key,
            condition,
            mutation,
            crate::WriteOptions::default(),
        )
    }

    /// Evaluate `condition` and apply `mutation` with explicit write options.
    ///
    /// # Errors
    ///
    /// Returns `crate::StorkvError::ScopeIdNotFound` when the scope is
    /// unknown, `crate::StorkvError::WriteStalled` in non-blocking mode when
    /// a backpressure watermark is exceeded, or a storage or WAL error.
    pub fn put_if_with_options(
        &self,
        scope_id: u32,
        user_key: Bytes,
        condition: &PutCondition,
        mutation: TxMutation,
        options: crate::WriteOptions,
    ) -> crate::Result<PutIfResult> {
        self.wait_for_write_capacity(&[scope_id], options)?;
        let (result, touched) = {
            let _commit_guard = self.commit_lock.lock();
            let now_ms = TimestampMs::now();
            let read_seq = self.last_visible_seq.load(Ordering::Acquire);
            let scope = self
                .scopes
                .read()
                .get(&scope_id)
                .cloned()
                .ok_or(crate::StorkvError::ScopeIdNotFound(scope_id))?;
            let read_sn = SequenceNumber::new(read_seq);
            let meta = scope.key_meta(&user_key, read_sn, now_ms)?;

            let (matches, current_value) = match condition {
                PutCondition::Absent => (meta.is_none(), None),
                PutCondition::Present => (meta.is_some(), None),
                PutCondition::ExpectedSeq(expected) => {
                    (meta.is_some_and(|meta| meta.seq == *expected), None)
                }
                PutCondition::ExpectedValue(expected) => {
                    let value = if meta.is_some() {
                        self.get_at(scope_id, &user_key, read_sn)?
                    } else {
                        None
                    };
                    let matched = value.as_ref() == Some(expected);
                    (matched, value)
                }
            };
            if !matches {
                let current_value = if current_value.is_none() && meta.is_some() {
                    self.get_at(scope_id, &user_key, read_sn)?
                } else {
                    current_value
                };
                let result = PutIfResult::Conflict {
                    current_seq: meta.map(|meta| meta.seq),
                    current_value,
                };
                return Ok(result);
            }

            let mut transaction = TransactionContext::new(read_seq);
            transaction.write_set.insert((scope_id, user_key), mutation);
            let touched = self.commit_tx_locked(transaction)?;
            let seq = self.last_visible_seq.load(Ordering::Acquire);
            (PutIfResult::Applied { seq }, touched)
        };

        for scope in touched {
            self.maybe_flush_memtable(&scope)?;
        }
        Ok(result)
    }
}
