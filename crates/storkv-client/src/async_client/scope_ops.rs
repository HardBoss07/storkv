use std::time::Duration;

use bytes::Bytes;
use storkv_proto::{
    Request, Response, WireDurability, WireMergeOp, WireRetention, WireScopeHandle,
    WireScopeOptions,
};

use crate::transport::{ClientResult, ScopeHandle};

use super::{AsyncClient, expect_empty, unexpected_response};

impl AsyncClient {
    /// Create a scope and return its handle.
    pub async fn create_scope<S: Into<String>>(
        &self,
        name: S,
        durability: WireDurability,
        retention: WireRetention,
        prefix_bloom_len: Option<usize>,
    ) -> ClientResult<ScopeHandle> {
        let options = WireScopeOptions {
            durability,
            retention,
            prefix_bloom_len,
            min_ttl: None,
            max_ttl: None,
            allow_permanent: true,
            active_ttl_tracking: false,
        };
        self.create_scope_with_options(name, options).await
    }

    /// Create a scope from a complete wire option set.
    pub async fn create_scope_with_options<S: Into<String>>(
        &self,
        name: S,
        options: WireScopeOptions,
    ) -> ClientResult<ScopeHandle> {
        match self
            .send_request(Request::CreateScope {
                name: name.into(),
                options,
            })
            .await?
        {
            Response::Scope(WireScopeHandle { scope_id, name }) => {
                Ok(ScopeHandle { scope_id, name })
            }
            other => Err(unexpected_response(other)),
        }
    }

    /// Drop a scope and delete its data.
    pub async fn drop_scope<S: AsRef<str>>(&self, name: S) -> ClientResult<()> {
        expect_empty(
            self.send_request(Request::DropScope {
                name: name.as_ref().to_string(),
            })
            .await?,
        )
    }

    /// Read one key. `None` means the key does not exist.
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
    pub async fn put_with_ttl<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
        ttl: Duration,
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

    /// Pin a snapshot at the current sequence number.
    pub async fn create_snapshot<S: Into<String>>(&self, name: S) -> ClientResult<()> {
        expect_empty(
            self.send_request(Request::CreateSnapshot { name: name.into() })
                .await?,
        )
    }

    /// Release a pinned snapshot.
    pub async fn unpin_snapshot<S: AsRef<str>>(&self, name: S) -> ClientResult<()> {
        expect_empty(
            self.send_request(Request::UnpinSnapshot {
                name: name.as_ref().to_string(),
            })
            .await?,
        )
    }

    /// Copy a consistent database image to a directory.
    pub async fn checkpoint<S: Into<String>>(&self, dest_dir: S) -> ClientResult<()> {
        expect_empty(
            self.send_request(Request::Checkpoint {
                dest_dir: dest_dir.into(),
            })
            .await?,
        )
    }

    /// Run compaction immediately.
    pub async fn compact(&self) -> ClientResult<()> {
        expect_empty(self.send_request(Request::Compact).await?)
    }
}
