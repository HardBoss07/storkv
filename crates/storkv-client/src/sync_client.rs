use std::time::Duration;

use bytes::Bytes;
use storkv_proto::{
    WireAdminStats, WireBackupEntry, WireBackupReport, WireBackupStatus, WireBackupVerifyReport,
    WireDurability, WireMergeOp, WireRetention, WireTelemetrySnapshot,
};

use crate::async_client::AsyncClient;
use crate::transport::{ClientResult, ScopeHandle};

/// Synchronous wrapper around AsyncClient for blocking environments
pub struct SyncClient {
    runtime: tokio::runtime::Runtime,
    inner: AsyncClient,
}

impl SyncClient {
    /// Connect over TCP (`host:port`).
    pub fn connect(address: &str) -> ClientResult<Self> {
        let runtime = build_runtime()?;
        let inner = runtime.block_on(AsyncClient::connect(address))?;
        Ok(Self { runtime, inner })
    }

    /// Connect to either a TCP endpoint or a Unix-domain socket path.
    pub fn connect_any(address: &str) -> ClientResult<Self> {
        let runtime = build_runtime()?;
        let inner = runtime.block_on(AsyncClient::connect_any(address))?;
        Ok(Self { runtime, inner })
    }

    /// Connect using any supported URI scheme (`storkv://`, `unix://`,
    /// `http://`, `ws://`, or `grpc://`).
    pub fn connect_uri(uri: &str) -> ClientResult<Self> {
        let runtime = build_runtime()?;
        let inner = runtime.block_on(AsyncClient::connect_uri(uri))?;
        Ok(Self { runtime, inner })
    }

    /// Send a ping and require a pong reply.
    pub fn ping(&self) -> ClientResult<()> {
        self.runtime.block_on(self.inner.ping())
    }

    /// Run a health check.
    pub fn health_check(&self) -> ClientResult<()> {
        self.runtime.block_on(self.inner.health_check())
    }

    /// Read visible-version metadata of a key.
    pub fn key_meta<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
    ) -> ClientResult<Option<storkv_proto::WireKeyMeta>> {
        self.runtime.block_on(self.inner.key_meta(scope, key))
    }

    /// Read runtime counters.
    pub fn telemetry(&self) -> ClientResult<WireTelemetrySnapshot> {
        self.runtime.block_on(self.inner.telemetry())
    }

    /// Read rich admin statistics.
    pub fn admin_stats(&self) -> ClientResult<WireAdminStats> {
        self.runtime.block_on(self.inner.admin_stats())
    }

    /// Run a full backup now.
    pub fn trigger_backup(&self, wait: bool) -> ClientResult<WireBackupReport> {
        self.runtime.block_on(self.inner.trigger_backup(wait))
    }

    /// Read the live backup worker status.
    pub fn backup_status(&self) -> ClientResult<WireBackupStatus> {
        self.runtime.block_on(self.inner.backup_status())
    }

    /// Verify archived backup generations.
    pub fn verify_backup(&self, target_seq: Option<u64>) -> ClientResult<WireBackupVerifyReport> {
        self.runtime.block_on(self.inner.verify_backup(target_seq))
    }

    /// List archived backup generations.
    pub fn list_backups(&self) -> ClientResult<Vec<WireBackupEntry>> {
        self.runtime.block_on(self.inner.list_backups())
    }

    /// Create a scope and return its handle.
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
    pub fn drop_scope<S: AsRef<str>>(&self, name: S) -> ClientResult<()> {
        self.runtime.block_on(self.inner.drop_scope(name))
    }

    /// Read one key. `None` means the key does not exist.
    pub fn get<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> ClientResult<Option<Bytes>> {
        self.runtime.block_on(self.inner.get(scope, key))
    }

    /// Store one key/value pair.
    pub fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
    ) -> ClientResult<()> {
        self.runtime.block_on(self.inner.put(scope, key, val))
    }

    /// Store one key/value pair with a time to live.
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
    pub fn delete<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> ClientResult<()> {
        self.runtime.block_on(self.inner.delete(scope, key))
    }

    /// Delete a half-open key range.
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
    pub fn merge<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        op: WireMergeOp,
    ) -> ClientResult<()> {
        self.runtime.block_on(self.inner.merge(scope, key, op))
    }

    /// Read an ordered key range. `None` bounds are open.
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
    pub fn scan_prefix<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        prefix: K,
    ) -> ClientResult<Vec<(Bytes, Bytes)>> {
        self.runtime.block_on(self.inner.scan_prefix(scope, prefix))
    }

    /// Copy a consistent database image to a directory.
    pub fn checkpoint<S: Into<String>>(&self, dest_dir: S) -> ClientResult<()> {
        self.runtime.block_on(self.inner.checkpoint(dest_dir))
    }
}

fn build_runtime() -> ClientResult<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}
