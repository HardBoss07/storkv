//! Explicit OCC transaction handle.

use crate::backend::{Conn, ResolvedScope};
use crate::error::{Result, SdkError};
use crate::pool::PoolConnection;
use crate::scope::Scope;

mod read;
mod write;

pub(crate) enum TxInner {
    Embedded(storkv_core::Transaction),
    Networked(storkv_client::AsyncTransaction),
}

/// Explicit OCC transaction created from a pooled connection.
///
/// Transactions buffer writes and validate their read set on
/// [`Transaction::commit`]. Dropping a transaction rolls it back.
pub struct Transaction {
    pub(super) inner: TxInner,
    pub(super) connection: Option<PoolConnection>,
}

impl Transaction {
    pub(crate) async fn begin(connection: PoolConnection) -> Result<Self> {
        let inner = match connection.conn()? {
            Conn::Embedded(db) => TxInner::Embedded(db.begin_tx()),
            Conn::Networked { client, .. } => TxInner::Networked(client.begin_tx().await?),
        };
        Ok(Self {
            inner,
            connection: Some(connection),
        })
    }

    pub(super) async fn resolve(&self, scope: &Scope) -> Result<ResolvedScope> {
        let connection = self
            .connection
            .as_ref()
            .ok_or(SdkError::PoolClosed)?
            .conn()?;
        scope.resolve_via(connection).await
    }
}

pub(super) fn resolved_core(scope: &ResolvedScope) -> storkv_core::ScopeHandle {
    storkv_core::ScopeHandle {
        scope_id: scope.scope_id,
        name: scope.name.clone(),
    }
}

pub(super) fn resolved_client(scope: &ResolvedScope) -> storkv_client::ScopeHandle {
    storkv_client::ScopeHandle {
        scope_id: scope.scope_id,
        name: scope.name.clone(),
    }
}
