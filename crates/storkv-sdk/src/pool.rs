//! Connection pool, checkout, and transaction helpers.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::Future;
use storkv_core::Db;

use crate::backend::NetworkedBackend;
use crate::cache::ScopeCache;
use crate::error::Result;
use crate::options::StorkvPoolOptions;
use crate::scope::Scope;

mod builder;
mod ops;
mod routing;

pub use builder::PoolConnection;

/// Boxed future returned by transaction closures.
pub type TxFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub(crate) enum Backend {
    Embedded(Db),
    Networked(Arc<NetworkedBackend>),
}

pub(crate) struct PoolInner {
    pub(crate) options: StorkvPoolOptions,
    pub(crate) backend: Backend,
    pub(crate) semaphore: Arc<tokio::sync::Semaphore>,
    pub(crate) caches: parking_lot::Mutex<HashMap<String, Arc<ScopeCache>>>,
    pub(crate) closed: AtomicBool,
}

/// Unified connection pool for embedded and networked StorKV backends.
///
/// A pool opened with a `file://` or `embedded://` URI shares one embedded
/// database handle. A pool opened with `storkv://`, `grpc://`, `http://`,
/// `https://`, `ws://`, `wss://`, or `unix://` keeps a set of independent
/// client connections.
#[derive(Clone)]
pub struct StorkvPool {
    inner: Arc<PoolInner>,
}

impl std::fmt::Debug for StorkvPool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StorkvPool")
            .field("options", &self.inner.options)
            .finish_non_exhaustive()
    }
}

impl StorkvPool {
    /// Return this pool's options.
    #[must_use]
    pub fn options(&self) -> &StorkvPoolOptions {
        &self.inner.options
    }

    pub(crate) fn inner_backend(&self) -> &Backend {
        &self.inner.backend
    }

    pub(crate) fn scope_cache(&self, name: &str) -> Arc<ScopeCache> {
        let mut caches = self.inner.caches.lock();
        caches
            .entry(name.to_string())
            .or_insert_with(|| ScopeCache::new(self.inner.options.cache))
            .clone()
    }

    /// Return `true` when the pool is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    /// Return a lightweight handle for the named scope.
    #[must_use]
    pub fn scope(&self, name: impl Into<String>) -> Scope {
        Scope::new(self.clone(), name.into())
    }
}
