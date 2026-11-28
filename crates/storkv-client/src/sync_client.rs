//! Blocking client wrapper that owns a current-thread Tokio runtime.
//!
//! Do not call a method from inside another Tokio runtime. The nested runtime
//! blocks the current thread.

use storkv_proto::{
    WireAdminStats, WireBackupEntry, WireBackupReport, WireBackupStatus, WireBackupVerifyReport,
    WireTelemetrySnapshot,
};

use crate::async_client::AsyncClient;
use crate::transport::{ClientResult, ScopeHandle};

mod data;

/// Synchronous wrapper around [`AsyncClient`] for blocking callers.
pub struct SyncClient {
    runtime: tokio::runtime::Runtime,
    inner: AsyncClient,
}

impl SyncClient {
    /// Connect over TCP (`host:port`).
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Io`] when the runtime cannot start or the socket fails.
    pub fn connect(address: &str) -> ClientResult<Self> {
        let runtime = build_runtime()?;
        let inner = runtime.block_on(AsyncClient::connect(address))?;
        Ok(Self { runtime, inner })
    }

    /// Connect to either a TCP endpoint or a Unix-domain socket path.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Protocol`] when the address is invalid.
    /// Returns [`crate::ClientError::Unsupported`] when the platform has no Unix sockets.
    /// Returns [`crate::ClientError::Io`] when the runtime cannot start or the socket fails.
    pub fn connect_any(address: &str) -> ClientResult<Self> {
        let runtime = build_runtime()?;
        let inner = runtime.block_on(AsyncClient::connect_any(address))?;
        Ok(Self { runtime, inner })
    }

    /// Connect using any supported URI scheme.
    ///
    /// The schemes are `storkv://`, `unix://`, `http://`, `ws://`, and `grpc://`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Protocol`] when the URI is invalid.
    /// Returns [`crate::ClientError::Unsupported`] when the platform has no Unix sockets.
    /// Returns [`crate::ClientError::Io`], [`crate::ClientError::Http`], [`crate::ClientError::Grpc`],
    /// or [`crate::ClientError::GrpcConnect`] when the connection fails.
    pub fn connect_uri(uri: &str) -> ClientResult<Self> {
        let runtime = build_runtime()?;
        let inner = runtime.block_on(AsyncClient::connect_uri(uri))?;
        Ok(Self { runtime, inner })
    }

    /// Send a ping and require a pong reply.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Protocol`] when the reply is not a pong.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn ping(&self) -> ClientResult<()> {
        self.runtime.block_on(self.inner.ping())
    }

    /// Run a health check.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Protocol`] when the reply is not healthy.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn health_check(&self) -> ClientResult<()> {
        self.runtime.block_on(self.inner.health_check())
    }

    /// Read visible-version metadata of a key.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Protocol`] when the reply is not key metadata.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn key_meta<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
    ) -> ClientResult<Option<storkv_proto::WireKeyMeta>> {
        self.runtime.block_on(self.inner.key_meta(scope, key))
    }

    /// Read runtime counters.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Protocol`] when the reply is not telemetry.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn telemetry(&self) -> ClientResult<WireTelemetrySnapshot> {
        self.runtime.block_on(self.inner.telemetry())
    }

    /// Read rich admin statistics.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Protocol`] when the reply is not admin statistics.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn admin_stats(&self) -> ClientResult<WireAdminStats> {
        self.runtime.block_on(self.inner.admin_stats())
    }

    /// Run a full backup now.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the backup fails.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not a backup report.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn trigger_backup(&self, wait: bool) -> ClientResult<WireBackupReport> {
        self.runtime.block_on(self.inner.trigger_backup(wait))
    }

    /// Read the live backup worker status.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Protocol`] when the reply is not a backup status.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn backup_status(&self) -> ClientResult<WireBackupStatus> {
        self.runtime.block_on(self.inner.backup_status())
    }

    /// Verify archived backup generations.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Server`] when the verification fails.
    /// Returns [`crate::ClientError::Protocol`] when the reply is not a verify report.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn verify_backup(&self, target_seq: Option<u64>) -> ClientResult<WireBackupVerifyReport> {
        self.runtime.block_on(self.inner.verify_backup(target_seq))
    }

    /// List archived backup generations.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ClientError::Protocol`] when the reply is not a backup catalog.
    /// Returns [`crate::ClientError::Io`] or [`crate::ClientError::Http`] when the send fails.
    pub fn list_backups(&self) -> ClientResult<Vec<WireBackupEntry>> {
        self.runtime.block_on(self.inner.list_backups())
    }
}

fn build_runtime() -> ClientResult<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}
