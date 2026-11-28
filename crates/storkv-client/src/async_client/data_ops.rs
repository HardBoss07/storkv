//! Data-plane calls for the asynchronous client.
//!
//! Each method sends one request for one scope and checks the reply variant.

use bytes::Bytes;
use storkv_proto::{Request, Response, WireMergeOp};

use crate::transport::{ClientResult, ScopeHandle};

use super::{AsyncClient, expect_empty, unexpected_response};

impl AsyncClient {
    /// Read one key. `None` means the key does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the scope is unknown.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not a value.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn get<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
    ) -> ClientResult<Option<Bytes>> {
        match self
            .send_request(Request::Get {
                scope_id: scope.scope_id,
                key: Bytes::copy_from_slice(key.as_ref()),
            })
            .await?
        {
            Response::Value(value) => Ok(value),
            other => Err(unexpected_response(other)),
        }
    }

    /// Store one key/value pair.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the write is rejected.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
    ) -> ClientResult<()> {
        expect_empty(
            self.send_request(Request::Put {
                scope_id: scope.scope_id,
                key: Bytes::copy_from_slice(key.as_ref()),
                val: Bytes::copy_from_slice(val.as_ref()),
            })
            .await?,
        )
    }

    /// Store one key/value pair with a time to live.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the write is rejected.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn put_with_ttl<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
        ttl: std::time::Duration,
    ) -> ClientResult<()> {
        expect_empty(
            self.send_request(Request::PutWithTtl {
                scope_id: scope.scope_id,
                key: Bytes::copy_from_slice(key.as_ref()),
                val: Bytes::copy_from_slice(val.as_ref()),
                ttl,
            })
            .await?,
        )
    }

    /// Delete one key.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the delete is rejected.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn delete<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> ClientResult<()> {
        expect_empty(
            self.send_request(Request::Delete {
                scope_id: scope.scope_id,
                key: Bytes::copy_from_slice(key.as_ref()),
            })
            .await?,
        )
    }

    /// Delete a half-open key range.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the delete is rejected.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn delete_range<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        start_key: K,
        end_key: K,
    ) -> ClientResult<()> {
        expect_empty(
            self.send_request(Request::DeleteRange {
                scope_id: scope.scope_id,
                start_key: Bytes::copy_from_slice(start_key.as_ref()),
                end_key: Bytes::copy_from_slice(end_key.as_ref()),
            })
            .await?,
        )
    }

    /// Apply a merge operand to one key.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the merge is rejected.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn merge<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        op: WireMergeOp,
    ) -> ClientResult<()> {
        expect_empty(
            self.send_request(Request::Merge {
                scope_id: scope.scope_id,
                key: Bytes::copy_from_slice(key.as_ref()),
                op,
            })
            .await?,
        )
    }

    /// Read an ordered key range. `None` bounds are open.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the scope is unknown.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not a scan result.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn scan<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        start_key: Option<K>,
        end_key: Option<K>,
    ) -> ClientResult<Vec<(Bytes, Bytes)>> {
        match self
            .send_request(Request::Scan {
                scope_id: scope.scope_id,
                start_key: start_key.map(|bound| Bytes::copy_from_slice(bound.as_ref())),
                end_key: end_key.map(|bound| Bytes::copy_from_slice(bound.as_ref())),
                limit: None,
                reverse: false,
                keys_only: false,
            })
            .await?
        {
            Response::Scan(entries) => Ok(entries),
            other => Err(unexpected_response(other)),
        }
    }

    /// Read every key with the given prefix.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the scope is unknown.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not a scan result.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn scan_prefix<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        prefix: K,
    ) -> ClientResult<Vec<(Bytes, Bytes)>> {
        match self
            .send_request(Request::ScanPrefix {
                scope_id: scope.scope_id,
                prefix: Bytes::copy_from_slice(prefix.as_ref()),
                limit: None,
                reverse: false,
                keys_only: false,
            })
            .await?
        {
            Response::Scan(entries) => Ok(entries),
            other => Err(unexpected_response(other)),
        }
    }
}
