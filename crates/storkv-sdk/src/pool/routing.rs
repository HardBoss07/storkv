//! Consistency, scope discovery, provisioning, and connection failover.

use std::time::Duration;

use storkv_client::AsyncClient;
use storkv_proto::Consistency;

use crate::backend::{NetworkedBackend, ResolvedScope};
use crate::error::{Result, SdkError};
use crate::options::ScopeDefinition;

use super::{PoolInner, StorkvPool};

const CONNECT_ATTEMPTS: u32 = 3;
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(25);
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

impl StorkvPool {
    /// Resolve the identifier of a scope by name.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::ScopeNotFound`] when the backend does not know the
    /// scope.
    pub(crate) async fn resolve_scope(&self, name: &str) -> Result<ResolvedScope> {
        let connection = self.acquire().await?;
        connection.conn()?.resolve_scope(name).await
    }

    /// Set the consistency level on the backend server.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::PoolClosed`] after [`StorkvPool::close`], or the
    /// request error.
    pub async fn set_consistency(&self, consistency: Consistency) -> Result<()> {
        if self.is_closed() {
            return Err(SdkError::PoolClosed);
        }
        let connection = self.acquire().await?;
        connection.conn()?.set_consistency(consistency).await
    }

    /// Idempotently create or verify a set of scopes.
    ///
    /// # Errors
    ///
    /// Returns an error when the pool cannot create or verify a scope.
    pub async fn provision_scopes(&self, definitions: &[ScopeDefinition]) -> Result<()> {
        let connection = self.acquire().await?;
        for definition in definitions {
            connection.conn()?.provision(definition).await?;
        }
        Ok(())
    }
}

pub(super) async fn connect_networked(
    backend: &NetworkedBackend,
    consistency: Consistency,
) -> Result<AsyncClient> {
    let mut last_error = None;
    for attempt in 0..CONNECT_ATTEMPTS {
        match AsyncClient::connect_uri(&backend.uri).await {
            Ok(client) => {
                if let Err(error) = client.set_consistency(consistency).await {
                    last_error = Some(SdkError::from(error));
                } else {
                    return Ok(client);
                }
            }
            Err(error) => {
                last_error = Some(SdkError::from(error));
            }
        }
        if attempt + 1 < CONNECT_ATTEMPTS {
            tokio::time::sleep(CONNECT_RETRY_DELAY).await;
        }
    }
    Err(last_error.unwrap_or_else(|| SdkError::InvalidUri(backend.uri.clone())))
}

pub(super) async fn take_networked_client(
    inner: &PoolInner,
    backend: &NetworkedBackend,
) -> Result<AsyncClient> {
    let mut idle = backend.idle.lock().await;
    while let Some(candidate) = idle.pop() {
        if let Some(limit) = inner.options.idle_timeout
            && candidate.last_used.elapsed() > limit
        {
            continue;
        }
        if inner.options.health_check {
            let healthy =
                tokio::time::timeout(HEALTH_CHECK_TIMEOUT, candidate.client.health_check())
                    .await
                    .map(|result| result.is_ok())
                    .unwrap_or(false);
            if healthy {
                return Ok(candidate.client);
            }
            continue;
        }
        return Ok(candidate.client);
    }
    drop(idle);
    connect_networked(backend, Consistency::Strong).await
}
