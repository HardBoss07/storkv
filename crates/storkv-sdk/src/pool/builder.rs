//! Pool construction, connection checkout, and connection return.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use storkv_core::{Db, DbOptions};

use crate::backend::{Conn, IdleClient, NetworkedBackend};
use crate::error::{Result, SdkError};
use crate::options::StorkvPoolOptions;

use super::routing::{connect_networked, take_networked_client};
use super::{Backend, PoolInner, StorkvPool};

impl StorkvPool {
    /// Open a pool with default options.
    ///
    /// # Errors
    ///
    /// Returns an error when the URI is invalid or the backend cannot open.
    pub async fn connect(uri: impl AsRef<str>) -> Result<Self> {
        Self::connect_with(uri.as_ref(), StorkvPoolOptions::default()).await
    }

    /// Open a pool with explicit options.
    ///
    /// # Errors
    ///
    /// Returns an error when the URI is invalid or the backend cannot open.
    pub async fn connect_with(uri: &str, options: StorkvPoolOptions) -> Result<Self> {
        options.validate()?;
        let (embedded_path, networked_uri) = parse_uri(uri)?;
        let backend = if let Some(path) = embedded_path {
            Backend::Embedded(Db::open(path, DbOptions::default())?)
        } else {
            let networked_uri =
                networked_uri.ok_or_else(|| SdkError::InvalidUri(uri.to_string()))?;
            let backend = Arc::new(NetworkedBackend::new(networked_uri));
            for _ in 0..options.min_connections {
                let client = connect_networked(&backend, options.consistency).await?;
                backend.idle.lock().await.push(IdleClient {
                    client,
                    last_used: Instant::now(),
                });
            }
            Backend::Networked(backend)
        };
        Ok(Self {
            inner: Arc::new(PoolInner {
                semaphore: Arc::new(tokio::sync::Semaphore::new(
                    options.max_connections as usize,
                )),
                options,
                backend,
                caches: parking_lot::Mutex::new(HashMap::new()),
                closed: AtomicBool::new(false),
            }),
        })
    }

    /// Close the pool and release idle connections.
    pub async fn close(&self) {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.semaphore.close();
        if let Backend::Networked(backend) = &self.inner.backend {
            backend.idle.lock().await.clear();
        }
    }

    /// Acquire one pooled connection.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::AcquireTimeout`] when no connection becomes
    /// available within the configured timeout, or
    /// [`SdkError::PoolClosed`] after [`StorkvPool::close`].
    pub async fn acquire(&self) -> Result<PoolConnection> {
        if self.is_closed() {
            return Err(SdkError::PoolClosed);
        }
        let permit = match tokio::time::timeout(
            self.inner.options.acquire_timeout,
            self.inner.semaphore.clone().acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => return Err(SdkError::PoolClosed),
            Err(_) => return Err(SdkError::AcquireTimeout),
        };
        let connection = match &self.inner.backend {
            Backend::Embedded(db) => Conn::Embedded(db.clone()),
            Backend::Networked(backend) => {
                let client = take_networked_client(&self.inner, backend).await?;
                Conn::Networked {
                    client,
                    backend: backend.clone(),
                }
            }
        };
        Ok(PoolConnection {
            inner: self.inner.clone(),
            connection: Some(connection),
            _permit: permit,
        })
    }
}

/// One checked-out connection. It returns to the pool when dropped.
pub struct PoolConnection {
    inner: Arc<PoolInner>,
    connection: Option<Conn>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl std::fmt::Debug for PoolConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PoolConnection")
            .finish_non_exhaustive()
    }
}

impl PoolConnection {
    pub(crate) fn conn(&self) -> Result<&Conn> {
        self.connection.as_ref().ok_or(SdkError::PoolClosed)
    }
}

impl Drop for PoolConnection {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };
        let Conn::Networked { client, backend } = connection else {
            return;
        };
        if self.inner.closed.load(Ordering::Acquire) {
            return;
        }
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                backend.idle.lock().await.push(IdleClient {
                    client,
                    last_used: Instant::now(),
                });
            });
        }
    }
}

fn parse_uri(uri: &str) -> Result<(Option<String>, Option<String>)> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return Err(SdkError::InvalidUri("empty connection URI".into()));
    }
    for scheme in ["file://", "embedded://"] {
        if let Some(path) = trimmed.strip_prefix(scheme) {
            if path.is_empty() {
                return Err(SdkError::InvalidUri(trimmed.to_string()));
            }
            return Ok((Some(path.to_string()), None));
        }
    }
    for scheme in [
        "storkv://",
        "grpc://",
        "http://",
        "https://",
        "ws://",
        "wss://",
        "unix://",
    ] {
        if trimmed.starts_with(scheme) {
            return Ok((None, Some(trimmed.to_string())));
        }
    }
    if trimmed.contains(':') {
        return Ok((None, Some(trimmed.to_string())));
    }
    Err(SdkError::InvalidUri(trimmed.to_string()))
}
