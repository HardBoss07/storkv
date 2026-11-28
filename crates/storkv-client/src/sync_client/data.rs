//! Blocking data-plane calls that forward to the asynchronous client.

use std::time::Duration;

use bytes::Bytes;
use storkv_proto::{WireDurability, WireMergeOp, WireRetention};

use crate::transport::{ClientResult, ScopeHandle};

use super::SyncClient;

impl SyncClient {
    /// Create a scope and return its handle.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the scope name already exists.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not a scope handle.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn create_scope<S: Into<String>>(
        &self,
        name: S,
        durability: WireDurability,
        retention: WireRetention,
        prefix_bloom_len: Option<usize>,
    ) -> ClientResult<ScopeHandle> {
        self.runtime.block_on(self.inner.create_scope(
            name,
            durability,
            retention,
            prefix_bloom_len,
        ))
    }

    /// Drop a scope and delete its data.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the scope does not exist.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn drop_scope<S: AsRef<str>>(&self, name: S) -> ClientResult<()> {
        self.runtime.block_on(self.inner.drop_scope(name))
    }

    /// Read one key. `None` means the key does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the scope is unknown.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not a value.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn get<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> ClientResult<Option<Bytes>> {
        self.runtime.block_on(self.inner.get(scope, key))
    }

    /// Store one key/value pair.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the write is rejected.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
    ) -> ClientResult<()> {
        self.runtime.block_on(self.inner.put(scope, key, val))
    }

    /// Store one key/value pair with a time to live.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the write is rejected.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn put_with_ttl<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
        ttl: Duration,
    ) -> ClientResult<()> {
        self.runtime
            .block_on(self.inner.put_with_ttl(scope, key, val, ttl))
    }

    /// Delete one key.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the delete is rejected.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn delete<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> ClientResult<()> {
        self.runtime.block_on(self.inner.delete(scope, key))
    }

    /// Delete a half-open key range.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the delete is rejected.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn delete_range<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        start_key: K,
        end_key: K,
    ) -> ClientResult<()> {
        self.runtime
            .block_on(self.inner.delete_range(scope, start_key, end_key))
    }

    /// Apply a merge operand to one key.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the merge is rejected.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn merge<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        op: WireMergeOp,
    ) -> ClientResult<()> {
        self.runtime.block_on(self.inner.merge(scope, key, op))
    }

    /// Read an ordered key range. `None` bounds are open.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the scope is unknown.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not a scan result.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn scan<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        start_key: Option<K>,
        end_key: Option<K>,
    ) -> ClientResult<Vec<(Bytes, Bytes)>> {
        self.runtime
            .block_on(self.inner.scan(scope, start_key, end_key))
    }

    /// Read every key with the given prefix.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the scope is unknown.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not a scan result.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn scan_prefix<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        prefix: K,
    ) -> ClientResult<Vec<(Bytes, Bytes)>> {
        self.runtime.block_on(self.inner.scan_prefix(scope, prefix))
    }

    /// Copy a consistent database image to a directory.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the checkpoint fails.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn checkpoint<S: Into<String>>(&self, dest_dir: S) -> ClientResult<()> {
        self.runtime.block_on(self.inner.checkpoint(dest_dir))
    }
}
