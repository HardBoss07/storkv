//! Key/value, range, and TTL operations of a database handle.

use std::sync::atomic::Ordering;
use std::time::Duration;

use bytes::Bytes;

use super::{Db, ScopeHandle};
use crate::ScanOptions;
use crate::WriteOptions;

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
        self.put_with_options(scope, key, val, WriteOptions::default())
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
        self.put_with_ttl_with_options(scope, key, val, ttl, WriteOptions::default())
    }

    /// Write or delete one key when `condition` matches the newest state.
    ///
    /// A `val` of `None` deletes the key. A `ttl` of `None` writes a permanent
    /// key. The condition is checked atomically with the write.
    ///
    /// # Errors
    ///
    /// Returns `crate::StorkvError::InvalidTtl` when the scope policy rejects
    /// the expiry, or an error when the write fails.
    pub fn put_if<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        condition: &crate::PutCondition,
        val: Option<V>,
        ttl: Option<Duration>,
    ) -> crate::Result<crate::PutIfResult> {
        self.put_if_with_options(scope, key, condition, val, ttl, WriteOptions::default())
    }

    pub(super) fn scope_options(&self, scope: &ScopeHandle) -> crate::Result<crate::ScopeOptions> {
        self.inner
            .scopes
            .read()
            .get(&scope.scope_id)
            .map(|scope| scope.effective_options())
            .ok_or(crate::StorkvError::ScopeIdNotFound(scope.scope_id))
    }

    /// Delete one key.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails.
    pub fn delete<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> crate::Result<()> {
        self.delete_with_options(scope, key, WriteOptions::default())
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
        self.merge_with_options(scope, key, op, WriteOptions::default())
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
        self.delete_range_with_options(scope, start_key, end_key, WriteOptions::default())
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
        self.scan_with(scope, start_key, end_key, ScanOptions::default())
    }

    /// Read an ordered key range with a limit, reverse order, or keys only.
    ///
    /// # Errors
    ///
    /// Returns an error when a storage read fails.
    pub fn scan_with<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        start_key: Option<K>,
        end_key: Option<K>,
        options: ScanOptions,
    ) -> crate::Result<Vec<(Bytes, Bytes)>> {
        self.inner.scan_with(
            scope.scope_id,
            start_key.as_ref().map(|key| key.as_ref()),
            end_key.as_ref().map(|key| key.as_ref()),
            self.inner.last_visible_seq.load(Ordering::Acquire),
            options,
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
        self.scan_prefix_with(scope, prefix, ScanOptions::default())
    }

    /// Read every key with the given prefix, with a limit, reverse order, or keys only.
    ///
    /// # Errors
    ///
    /// Returns an error when a storage read fails.
    pub fn scan_prefix_with<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        prefix: K,
        options: ScanOptions,
    ) -> crate::Result<Vec<(Bytes, Bytes)>> {
        self.inner.scan_prefix_with(
            scope.scope_id,
            prefix.as_ref(),
            self.inner.last_visible_seq.load(Ordering::Acquire),
            options,
        )
    }
}
