//! Key/value write operations with explicit write options.

use std::time::Duration;

use bytes::Bytes;

use super::{Db, ScopeHandle};
use crate::WriteOptions;

impl Db {
    /// Store one key/value pair with explicit write options.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails or a stop watermark is exceeded
    /// in non-blocking mode.
    pub fn put_with_options<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
        options: WriteOptions,
    ) -> crate::Result<()> {
        self.scope_options(scope)?
            .validate_expiry(0, crate::types::TimestampMs::now().get())?;
        let mut tx = self.begin_tx();
        tx.put(scope, key, val)?;
        tx.commit_with_options(options)
    }

    /// Store one key/value pair with a time to live and explicit write options.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails or a stop watermark is exceeded
    /// in non-blocking mode.
    pub fn put_with_ttl_with_options<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
        ttl: Duration,
        options: WriteOptions,
    ) -> crate::Result<()> {
        let now_ms = crate::types::TimestampMs::now().get();
        let expires_at_ms =
            now_ms.saturating_add(u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX));
        self.scope_options(scope)?
            .validate_expiry(expires_at_ms, now_ms)?;
        let mut tx = self.begin_tx();
        tx.put_with_expires_at(scope, key, val, expires_at_ms)?;
        tx.commit_with_options(options)
    }

    /// Write or delete one key when `condition` matches, with write options.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorkvError::InvalidTtl`] when the scope policy rejects
    /// the expiry, [`crate::StorkvError::WriteStalled`] in non-blocking mode
    /// when a stop watermark is exceeded, or an error when the write fails.
    pub fn put_if_with_options<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        condition: &crate::PutCondition,
        val: Option<V>,
        ttl: Option<Duration>,
        options: WriteOptions,
    ) -> crate::Result<crate::PutIfResult> {
        let now_ms = crate::types::TimestampMs::now().get();
        let mutation = match val {
            Some(val) => {
                let expires_at_ms = match ttl {
                    Some(ttl) => {
                        now_ms.saturating_add(u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX))
                    }
                    None => 0,
                };
                self.scope_options(scope)?
                    .validate_expiry(expires_at_ms, now_ms)?;
                crate::mvcc::TxMutation::Put {
                    val: Bytes::copy_from_slice(val.as_ref()),
                    expires_at_ms,
                }
            }
            None => crate::mvcc::TxMutation::Delete,
        };
        self.inner.put_if_with_options(
            scope.scope_id,
            Bytes::copy_from_slice(key.as_ref()),
            condition,
            mutation,
            options,
        )
    }

    /// Delete one key with explicit write options.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails or a stop watermark is exceeded
    /// in non-blocking mode.
    pub fn delete_with_options<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        options: WriteOptions,
    ) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.delete(scope, key)?;
        tx.commit_with_options(options)
    }

    /// Apply a merge operand to one key with explicit write options.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails or a stop watermark is exceeded
    /// in non-blocking mode.
    pub fn merge_with_options<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        op: crate::MergeOp,
        options: WriteOptions,
    ) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.merge(scope, key, op)?;
        tx.commit_with_options(options)
    }

    /// Delete a half-open key range with explicit write options.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails or a stop watermark is exceeded
    /// in non-blocking mode.
    pub fn delete_range_with_options<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        start_key: K,
        end_key: K,
        options: WriteOptions,
    ) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.delete_range(scope, start_key, end_key)?;
        tx.commit_with_options(options)
    }
}
