use std::time::Duration;

use storkv_proto::Consistency;

use crate::error::{Result, SdkError};
use crate::pool::StorkvPool;

use super::{CacheOptions, RetryPolicy};

/// Configuration for a [`StorkvPool`].
#[derive(Debug, Clone)]
pub struct StorkvPoolOptions {
    /// Probe idle connections before handing them out.
    pub health_check: bool,
    /// Maximum number of simultaneous connections.
    pub max_connections: u32,
    /// Number of connections opened eagerly.
    pub min_connections: u32,
    /// Maximum time to wait for an available connection.
    pub acquire_timeout: Duration,
    /// Discard idle connections older than this duration. `None` keeps them.
    pub idle_timeout: Option<Duration>,
    /// Default retry policy for transactions.
    pub retry: RetryPolicy,
    /// Optional client-side read cache.
    pub cache: CacheOptions,
    /// Consistency level that networked pools request from the server.
    pub consistency: Consistency,
}

impl Default for StorkvPoolOptions {
    fn default() -> Self {
        Self {
            health_check: true,
            max_connections: 10,
            min_connections: 1,
            acquire_timeout: Duration::from_secs(5),
            idle_timeout: Some(Duration::from_secs(300)),
            retry: RetryPolicy::default(),
            cache: CacheOptions::default(),
            consistency: Consistency::Strong,
        }
    }
}

impl StorkvPoolOptions {
    /// Create options with the documented defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum number of simultaneous connections.
    pub fn max_connections(mut self, max_connections: u32) -> Self {
        self.max_connections = max_connections;
        self
    }

    /// Set the number of connections opened eagerly.
    pub fn min_connections(mut self, min_connections: u32) -> Self {
        self.min_connections = min_connections;
        self
    }

    /// Set the maximum time to wait for an available connection.
    pub fn acquire_timeout(mut self, acquire_timeout: Duration) -> Self {
        self.acquire_timeout = acquire_timeout;
        self
    }

    /// Set how long an idle connection is kept. `None` keeps it indefinitely.
    pub fn idle_timeout(mut self, idle_timeout: Option<Duration>) -> Self {
        self.idle_timeout = idle_timeout;
        self
    }

    /// Enable or disable the health probe for idle connections.
    pub fn health_check(mut self, health_check: bool) -> Self {
        self.health_check = health_check;
        self
    }

    /// Set the default retry policy for transactions.
    pub fn retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Set the client-side cache configuration.
    pub fn cache(mut self, cache: CacheOptions) -> Self {
        self.cache = cache;
        self
    }

    /// Set the consistency level that networked pools request.
    pub fn consistency(mut self, consistency: Consistency) -> Self {
        self.consistency = consistency;
        self
    }

    /// Open a pool with these options.
    ///
    /// # Errors
    ///
    /// Returns an error when the URI is invalid or the backend cannot open.
    pub async fn connect(self, uri: impl AsRef<str>) -> Result<StorkvPool> {
        StorkvPool::connect_with(uri.as_ref(), self).await
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.max_connections == 0 {
            return Err(SdkError::InvalidUri(
                "max_connections must be at least 1".into(),
            ));
        }
        if self.min_connections > self.max_connections {
            return Err(SdkError::InvalidUri(
                "min_connections cannot exceed max_connections".into(),
            ));
        }
        Ok(())
    }
}
