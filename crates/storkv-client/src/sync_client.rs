use std::time::Duration;

use bytes::Bytes;
use storkv_proto::{
    WireAdminStats, WireDurability, WireMergeOp, WireRetention, WireTelemetrySnapshot,
};

use crate::async_client::AsyncClient;
use crate::transport::{ClientResult, ScopeHandle};

/// Synchronous wrapper around AsyncClient for blocking environments
pub struct SyncClient {
    runtime: tokio::runtime::Runtime,
    inner: AsyncClient,
}

impl SyncClient {
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

    pub fn ping(&self) -> ClientResult<()> {
        self.runtime.block_on(self.inner.ping())
    }

    pub fn health_check(&self) -> ClientResult<()> {
        self.runtime.block_on(self.inner.health_check())
    }

    pub fn telemetry(&self) -> ClientResult<WireTelemetrySnapshot> {
        self.runtime.block_on(self.inner.telemetry())
    }

    pub fn admin_stats(&self) -> ClientResult<WireAdminStats> {
        self.runtime.block_on(self.inner.admin_stats())
    }

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

    pub fn drop_scope<S: AsRef<str>>(&self, name: S) -> ClientResult<()> {
        self.runtime.block_on(self.inner.drop_scope(name))
    }

    pub fn get<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> ClientResult<Option<Bytes>> {
        self.runtime.block_on(self.inner.get(scope, key))
    }

    pub fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
    ) -> ClientResult<()> {
        self.runtime.block_on(self.inner.put(scope, key, val))
    }

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

    pub fn delete<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> ClientResult<()> {
        self.runtime.block_on(self.inner.delete(scope, key))
    }

    pub fn delete_range<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        start_key: K,
        end_key: K,
    ) -> ClientResult<()> {
        self.runtime
            .block_on(self.inner.delete_range(scope, start_key, end_key))
    }

    pub fn merge<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        op: WireMergeOp,
    ) -> ClientResult<()> {
        self.runtime.block_on(self.inner.merge(scope, key, op))
    }

    pub fn scan<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        start_key: Option<K>,
        end_key: Option<K>,
    ) -> ClientResult<Vec<(Bytes, Bytes)>> {
        self.runtime
            .block_on(self.inner.scan(scope, start_key, end_key))
    }

    pub fn scan_prefix<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        prefix: K,
    ) -> ClientResult<Vec<(Bytes, Bytes)>> {
        self.runtime.block_on(self.inner.scan_prefix(scope, prefix))
    }

    pub fn checkpoint<S: Into<String>>(&self, dest_dir: S) -> ClientResult<()> {
        self.runtime.block_on(self.inner.checkpoint(dest_dir))
    }
}

fn build_runtime() -> ClientResult<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}
