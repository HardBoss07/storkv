//! Scope, snapshot, checkpoint, and compaction calls for the asynchronous client.
//!
//! Scope handles carry the server scope identifier. Reuse one handle for one
//! scope for the life of the connection.

use bytes::Bytes;
use storkv_proto::{
    Request, Response, WireDurability, WireRetention, WireScopeHandle, WireScopeOptions,
};

use crate::transport::{ClientResult, ScopeHandle};

use super::{AsyncClient, expect_empty, unexpected_response};

impl AsyncClient {
    /// Create a scope and return its handle.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the scope name already exists.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not a scope handle.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
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
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the scope name already exists.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not a scope handle.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
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
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the scope does not exist.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn drop_scope<S: AsRef<str>>(&self, name: S) -> ClientResult<()> {
        expect_empty(
            self.send_request(Request::DropScope {
                name: name.as_ref().to_string(),
            })
            .await?,
        )
    }

    /// Pin a snapshot at the current sequence number.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the snapshot name is in use.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn create_snapshot<S: Into<String>>(&self, name: S) -> ClientResult<()> {
        expect_empty(
            self.send_request(Request::CreateSnapshot { name: name.into() })
                .await?,
        )
    }

    /// Release a pinned snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the snapshot does not exist.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn unpin_snapshot<S: AsRef<str>>(&self, name: S) -> ClientResult<()> {
        expect_empty(
            self.send_request(Request::UnpinSnapshot {
                name: name.as_ref().to_string(),
            })
            .await?,
        )
    }

    /// Copy a consistent database image to a directory.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the checkpoint fails.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn checkpoint<S: Into<String>>(&self, dest_dir: S) -> ClientResult<()> {
        expect_empty(
            self.send_request(Request::Checkpoint {
                dest_dir: dest_dir.into(),
            })
            .await?,
        )
    }

    /// Run compaction immediately.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the compaction fails.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn compact(&self) -> ClientResult<()> {
        expect_empty(self.send_request(Request::Compact).await?)
    }

    /// Compact only the SSTables that overlap `[start, end)` in one scope.
    ///
    /// `None` bounds are open. SSTables outside the range stay untouched.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the scope is unknown.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not empty.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub async fn compact_range(
        &self,
        scope: &str,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
    ) -> ClientResult<()> {
        expect_empty(
            self.send_request(Request::CompactRange {
                scope: scope.to_string(),
                start: start.map(Bytes::copy_from_slice),
                end: end.map(Bytes::copy_from_slice),
            })
            .await?,
        )
    }
}
