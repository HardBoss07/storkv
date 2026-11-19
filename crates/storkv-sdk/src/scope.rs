//! Lightweight scope handle with lazy identifier resolution.

use std::sync::Arc;

use tokio::sync::OnceCell;

use crate::backend::ResolvedScope;
use crate::error::Result;
use crate::pool::StorkvPool;

mod cached;
mod ops;

/// Lightweight handle to one scope.
///
/// The handle resolves the scope identifier on first use. It caches the result
/// for the lifetime of the handle.
#[derive(Clone)]
pub struct Scope {
    pool: StorkvPool,
    cache: Arc<crate::cache::ScopeCache>,
    name: String,
    resolved: Arc<OnceCell<ResolvedScope>>,
}

impl std::fmt::Debug for Scope {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Scope")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Scope {
    pub(crate) fn new(pool: StorkvPool, name: String) -> Self {
        let cache = pool.scope_cache(&name);
        Self {
            pool,
            cache,
            name,
            resolved: Arc::new(OnceCell::new()),
        }
    }

    /// Return the scope name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the pool that owns this scope handle.
    #[must_use]
    pub fn pool(&self) -> &StorkvPool {
        &self.pool
    }

    pub(crate) async fn resolve(&self) -> Result<ResolvedScope> {
        self.resolved
            .get_or_try_init(|| async { self.pool.resolve_scope(&self.name).await })
            .await
            .cloned()
    }

    pub(crate) async fn resolve_via(
        &self,
        connection: &crate::backend::Conn,
    ) -> Result<ResolvedScope> {
        self.resolved
            .get_or_try_init(|| async { connection.resolve_scope(&self.name).await })
            .await
            .cloned()
    }
}
