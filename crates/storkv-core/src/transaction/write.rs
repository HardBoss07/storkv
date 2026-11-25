//! Buffered writes and commit for an optimistic transaction.

use std::time::Duration;

use bytes::Bytes;

use crate::format::{MAX_KEY_LEN, MAX_VAL_LEN};
use crate::mvcc::{TransactionContext, TxMutation};
use crate::types::TimestampMs;
use crate::{MergeOp, StorkvError};

use super::Transaction;
use crate::vfs::FileSystem;

impl<FS: FileSystem + 'static> Transaction<FS> {
    /// Buffer a value write.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::KeyTooLarge`] or [`StorkvError::ValueTooLarge`]
    /// when a field exceeds the format limit.
    pub fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        key: K,
        val: V,
    ) -> crate::Result<()> {
        let user_key = key.as_ref();
        let entry_value = val.as_ref();
        validate_key(user_key)?;
        validate_value(entry_value)?;
        self.ctx.put(
            scope.scope_id,
            Bytes::copy_from_slice(user_key),
            Bytes::copy_from_slice(entry_value),
            0,
        );
        Ok(())
    }

    /// Buffer a value write with a time to live.
    ///
    /// # Errors
    ///
    /// Returns an error when the key or value exceeds the format limit.
    pub fn put_with_ttl<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        key: K,
        val: V,
        ttl: Duration,
    ) -> crate::Result<()> {
        let expires_at_ms =
            TimestampMs::now().get() + u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX);
        self.put_with_expires_at(scope, key, val, expires_at_ms)
    }

    /// Buffer a write with an absolute expiry timestamp (`0` = no expiry).
    pub fn put_with_expires_at<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        key: K,
        val: V,
        expires_at_ms: u64,
    ) -> crate::Result<()> {
        let user_key = key.as_ref();
        let entry_value = val.as_ref();
        validate_key(user_key)?;
        validate_value(entry_value)?;
        self.ctx.put(
            scope.scope_id,
            Bytes::copy_from_slice(user_key),
            Bytes::copy_from_slice(entry_value),
            expires_at_ms,
        );
        Ok(())
    }

    /// Track a key in the OCC read set without reading its value.
    pub fn record_read(&mut self, scope_id: u32, key: &[u8]) {
        self.ctx.record_read(scope_id, Bytes::copy_from_slice(key));
    }

    /// Buffer a delete.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::KeyTooLarge`] when the key exceeds the format limit.
    pub fn delete<K: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        key: K,
    ) -> crate::Result<()> {
        let user_key = key.as_ref();
        validate_key(user_key)?;
        self.ctx
            .delete(scope.scope_id, Bytes::copy_from_slice(user_key));
        Ok(())
    }

    /// Buffer a half-open range delete.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::KeyTooLarge`] when a bound exceeds the format limit.
    pub fn delete_range<K: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        start_key: K,
        end_key: K,
    ) -> crate::Result<()> {
        let range_start = start_key.as_ref();
        let range_end = end_key.as_ref();
        validate_key(range_start)?;
        validate_key(range_end)?;
        self.ctx.write_set.insert(
            (scope.scope_id, Bytes::copy_from_slice(range_start)),
            TxMutation::DeleteRange {
                end_key: Bytes::copy_from_slice(range_end),
            },
        );
        Ok(())
    }

    /// Buffer a merge operand.
    ///
    /// # Errors
    ///
    /// This method returns an error only for interface parity; it cannot fail.
    pub fn merge<K: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        key: K,
        op: MergeOp,
    ) -> crate::Result<()> {
        self.ctx.merge(
            scope.scope_id,
            Bytes::copy_from_slice(key.as_ref()),
            op.to_operand_bytes(),
        );
        Ok(())
    }

    /// Validate the read set and commit the write set.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::WriteConflict`] when a key in the read set changed
    /// after the transaction started.
    pub fn commit(self) -> crate::Result<()> {
        self.commit_with_options(crate::WriteOptions::default())
    }

    /// Validate the read set and commit the write set with explicit options.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::WriteConflict`] when a key in the read set changed
    /// after the transaction started, or
    /// [`StorkvError::WriteStalled`] in non-blocking mode when a backpressure
    /// watermark is exceeded.
    pub fn commit_with_options(mut self, options: crate::WriteOptions) -> crate::Result<()> {
        let read_seq = self.ctx.read_seq;
        let context = std::mem::replace(&mut self.ctx, TransactionContext::new(0));
        let result = self.db.commit_tx_with_options(context, options);
        self.release_reader(read_seq);
        result
    }

    pub(super) fn release_reader(&self, read_seq: u64) {
        let mut readers = self.db.active_tx_readers.write();
        if let Some(position) = readers.iter().position(|&seq| seq == read_seq) {
            readers.remove(position);
        }
    }
}

fn validate_key(user_key: &[u8]) -> crate::Result<()> {
    if user_key.len() > MAX_KEY_LEN {
        return Err(StorkvError::KeyTooLarge {
            max: MAX_KEY_LEN,
            actual: user_key.len(),
        });
    }
    Ok(())
}

fn validate_value(entry_value: &[u8]) -> crate::Result<()> {
    if entry_value.len() > MAX_VAL_LEN {
        return Err(StorkvError::ValueTooLarge {
            max: MAX_VAL_LEN,
            actual: entry_value.len(),
        });
    }
    Ok(())
}
