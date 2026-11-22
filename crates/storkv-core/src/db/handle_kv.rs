//! Key/value, range, and TTL operations of a database handle.

use std::sync::atomic::Ordering;
use std::time::Duration;

use bytes::Bytes;

use super::{Db, ScopeHandle};

impl Db {
    /// Read one key from the newest visible state.
    ///
    /// # Errors
    ///
    /// Returns an error when a storage read fails.
    pub fn get<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> crate::Result<Option<Bytes>> {
        self.inner.get(scope.scope_id, key.as_ref())
    }

    /// Store one key/value pair.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails.
    pub fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
    ) -> crate::Result<()> {
        self.scope_options(scope)?
            .validate_expiry(0, crate::types::TimestampMs::now().get())?;
        let mut tx = self.begin_tx();
        tx.put(scope, key, val)?;
        tx.commit()
    }

    /// Store one key/value pair with a time to live.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails.
    pub fn put_with_ttl<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
        ttl: Duration,
    ) -> crate::Result<()> {
        let now_ms = crate::types::TimestampMs::now().get();
        let expires_at_ms =
            now_ms.saturating_add(u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX));
        self.scope_options(scope)?
            .validate_expiry(expires_at_ms, now_ms)?;
        let mut tx = self.begin_tx();
        tx.put_with_expires_at(scope, key, val, expires_at_ms)?;
        tx.commit()
    }

    /// Write or delete one key when `condition` matches the newest state.
    ///
    /// A `val` of `None` deletes the key. A `ttl` of `None` writes a permanent
    /// key. The condition is checked atomically with the write.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorkvError::InvalidTtl`] when the scope policy rejects
    /// the expiry, or an error when the write fails.
    pub fn put_if<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        condition: &crate::PutCondition,
        val: Option<V>,
        ttl: Option<Duration>,
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
        self.inner.put_if(
            scope.scope_id,
            Bytes::copy_from_slice(key.as_ref()),
            condition,
            mutation,
        )
    }

    fn scope_options(&self, scope: &ScopeHandle) -> crate::Result<crate::ScopeOptions> {
        self.inner
            .scopes
            .read()
            .get(&scope.scope_id)
            .map(|scope| scope.options.clone())
            .ok_or(crate::StorkvError::ScopeIdNotFound(scope.scope_id))
    }

    /// Delete one key.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails.
    pub fn delete<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.delete(scope, key)?;
        tx.commit()
    }

    /// Apply a merge operand to one key.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails.
    pub fn merge<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        op: crate::MergeOp,
    ) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.merge(scope, key, op)?;
        tx.commit()
    }

    /// Delete a half-open key range.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails.
    pub fn delete_range<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        start_key: K,
        end_key: K,
    ) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.delete_range(scope, start_key, end_key)?;
        tx.commit()
    }

    /// Read an ordered key range. `None` bounds are open.
    ///
    /// # Errors
    ///
    /// Returns an error when a storage read fails.
    pub fn scan<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        start_key: Option<K>,
        end_key: Option<K>,
    ) -> crate::Result<Vec<(Bytes, Bytes)>> {
        self.inner.scan(
            scope.scope_id,
            start_key.as_ref().map(|key| key.as_ref()),
            end_key.as_ref().map(|key| key.as_ref()),
            self.inner.last_visible_seq.load(Ordering::Acquire),
        )
    }

    /// Read every key with the given prefix.
    ///
    /// # Errors
    ///
    /// Returns an error when a storage read fails.
    pub fn scan_prefix<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        prefix: K,
    ) -> crate::Result<Vec<(Bytes, Bytes)>> {
        self.inner.scan_prefix(
            scope.scope_id,
            prefix.as_ref(),
            self.inner.last_visible_seq.load(Ordering::Acquire),
        )
    }

    /// Metadata (absolute expiry and sequence) of the visible version of a key.
    pub fn key_meta(
        &self,
        scope: &ScopeHandle,
        key: impl AsRef<[u8]>,
    ) -> crate::Result<Option<crate::scope::KeyMeta>> {
        let read_seq = self.inner.last_visible_seq.load(Ordering::Acquire);
        let scope_state = self
            .inner
            .scopes
            .read()
            .get(&scope.scope_id)
            .cloned()
            .ok_or(crate::StorkvError::ScopeIdNotFound(scope.scope_id))?;
        scope_state.key_meta(
            key.as_ref(),
            crate::types::SequenceNumber::new(read_seq),
            crate::types::TimestampMs::now(),
        )
    }

    /// Remaining TTL in milliseconds.
    ///
    /// `None` means the key does not exist. `Some(0)` means the key is
    /// permanent; any other value is the exact remaining lifetime.
    pub fn key_ttl_remaining_ms(
        &self,
        scope: &ScopeHandle,
        key: impl AsRef<[u8]>,
    ) -> crate::Result<Option<u64>> {
        let Some(meta) = self.key_meta(scope, key)? else {
            return Ok(None);
        };
        if meta.expires_at_ms == 0 {
            return Ok(Some(0));
        }
        let now_ms = crate::types::TimestampMs::now().get();
        Ok(Some(meta.expires_at_ms.saturating_sub(now_ms)))
    }
}
