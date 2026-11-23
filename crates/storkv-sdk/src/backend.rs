use std::collections::HashMap;
use std::time::Instant;

use bytes::Bytes;
use storkv_client::AsyncClient;
use storkv_core::Db;
use storkv_proto::{Consistency, WireKeyMeta};
use tokio::sync::Mutex;

mod embedded;
mod remote;

use crate::error::Result;
use crate::options::ScopeDefinition;

/// Scope identifier resolved against one backend.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedScope {
    pub(crate) scope_id: u32,
    pub(crate) name: String,
}

impl ResolvedScope {
    fn core_handle(&self) -> storkv_core::ScopeHandle {
        storkv_core::ScopeHandle {
            scope_id: self.scope_id,
            name: self.name.clone(),
        }
    }

    fn client_handle(&self) -> storkv_client::ScopeHandle {
        storkv_client::ScopeHandle {
            scope_id: self.scope_id,
            name: self.name.clone(),
        }
    }
}

/// One idle networked connection.
pub(crate) struct IdleClient {
    pub(crate) last_used: Instant,
    pub(crate) client: AsyncClient,
}

/// Shared state for a networked pool.
pub(crate) struct NetworkedBackend {
    pub(crate) uri: String,
    pub(crate) idle: Mutex<Vec<IdleClient>>,
    pub(crate) scopes: Mutex<HashMap<String, u32>>,
}

impl NetworkedBackend {
    pub(crate) fn new(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            idle: Mutex::new(Vec::new()),
            scopes: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn remember_scope(&self, name: &str, scope_id: u32) {
        self.scopes.lock().await.insert(name.to_string(), scope_id);
    }
}

/// A checked-out backend connection.
pub(crate) enum Conn {
    Embedded(Db),
    Networked {
        client: AsyncClient,
        backend: std::sync::Arc<NetworkedBackend>,
    },
}

impl Conn {
    pub(crate) async fn set_consistency(&self, consistency: Consistency) -> Result<()> {
        match self {
            Conn::Embedded(_) => Ok(()),
            Conn::Networked { client, .. } => remote::set_consistency(client, consistency).await,
        }
    }

    pub(crate) async fn get(&self, scope: &ResolvedScope, key: &[u8]) -> Result<Option<Bytes>> {
        match self {
            Conn::Embedded(db) => embedded::get(db, scope, key).await,
            Conn::Networked { client, .. } => remote::get(client, scope, key).await,
        }
    }

    pub(crate) async fn key_meta(
        &self,
        scope: &ResolvedScope,
        key: &[u8],
    ) -> Result<Option<WireKeyMeta>> {
        match self {
            Conn::Embedded(db) => embedded::key_meta(db, scope, key).await,
            Conn::Networked { client, .. } => remote::key_meta(client, scope, key).await,
        }
    }

    pub(crate) async fn put(&self, scope: &ResolvedScope, key: &[u8], value: Bytes) -> Result<()> {
        match self {
            Conn::Embedded(db) => embedded::put(db, scope, key, value).await,
            Conn::Networked { client, .. } => remote::put(client, scope, key, value).await,
        }
    }

    pub(crate) async fn put_with_ttl(
        &self,
        scope: &ResolvedScope,
        key: &[u8],
        value: Bytes,
        ttl: std::time::Duration,
    ) -> Result<()> {
        match self {
            Conn::Embedded(db) => embedded::put_with_ttl(db, scope, key, value, ttl).await,
            Conn::Networked { client, .. } => {
                remote::put_with_ttl(client, scope, key, value, ttl).await
            }
        }
    }

    pub(crate) async fn put_if(
        &self,
        scope: &ResolvedScope,
        key: &[u8],
        condition: &storkv_core::PutCondition,
        value: Bytes,
        ttl: Option<std::time::Duration>,
    ) -> Result<storkv_core::PutIfResult> {
        match self {
            Conn::Embedded(db) => embedded::put_if(db, scope, key, condition, value, ttl).await,
            Conn::Networked { client, .. } => {
                remote::put_if(client, scope, key, condition, value, ttl).await
            }
        }
    }

    pub(crate) async fn merge(
        &self,
        scope: &ResolvedScope,
        key: &[u8],
        op: &storkv_core::MergeOp,
    ) -> Result<()> {
        match self {
            Conn::Embedded(db) => embedded::merge(db, scope, key, op).await,
            Conn::Networked { client, .. } => remote::merge(client, scope, key, op).await,
        }
    }

    pub(crate) async fn delete(&self, scope: &ResolvedScope, key: &[u8]) -> Result<()> {
        match self {
            Conn::Embedded(db) => embedded::delete(db, scope, key).await,
            Conn::Networked { client, .. } => remote::delete(client, scope, key).await,
        }
    }

    pub(crate) async fn scan_prefix(
        &self,
        scope: &ResolvedScope,
        prefix: &[u8],
    ) -> Result<Vec<(Bytes, Bytes)>> {
        match self {
            Conn::Embedded(db) => embedded::scan_prefix(db, scope, prefix).await,
            Conn::Networked { client, .. } => remote::scan_prefix(client, scope, prefix).await,
        }
    }

    pub(crate) async fn scan_range(
        &self,
        scope: &ResolvedScope,
        start: Option<Bytes>,
        end: Option<Bytes>,
    ) -> Result<Vec<(Bytes, Bytes)>> {
        match self {
            Conn::Embedded(db) => embedded::scan_range(db, scope, start, end).await,
            Conn::Networked { client, .. } => remote::scan_range(client, scope, start, end).await,
        }
    }

    pub(crate) async fn resolve_scope(&self, name: &str) -> Result<ResolvedScope> {
        match self {
            Conn::Embedded(db) => embedded::resolve_scope(db, name).await,
            Conn::Networked { client, backend } => {
                remote::resolve_scope(client, backend, name).await
            }
        }
    }

    pub(crate) async fn provision(&self, definition: &ScopeDefinition) -> Result<()> {
        match self {
            Conn::Embedded(db) => embedded::provision(db, definition).await,
            Conn::Networked { client, backend } => {
                remote::provision(client, backend, definition).await
            }
        }
    }
}

/// Convert a core merge operation to its wire form.
pub(crate) fn wire_merge_op(op: &storkv_core::MergeOp) -> Result<storkv_proto::WireMergeOp> {
    use storkv_core::MergeOp;
    Ok(match op {
        MergeOp::CounterAdd(delta) => storkv_proto::WireMergeOp::CounterAdd(*delta),
        MergeOp::Custom(operand) => storkv_proto::WireMergeOp::Custom(operand.clone()),
        MergeOp::AppendBytes { data, max_len } => storkv_proto::WireMergeOp::AppendBytes {
            data: data.clone(),
            max_len: max_len.map(|max_len| max_len as u64),
        },
        MergeOp::MaxI64(value) => storkv_proto::WireMergeOp::MaxI64(*value),
        MergeOp::MinI64(value) => storkv_proto::WireMergeOp::MinI64(*value),
        MergeOp::BitOr(bits) => storkv_proto::WireMergeOp::BitOr(*bits),
        MergeOp::BitAnd(bits) => storkv_proto::WireMergeOp::BitAnd(*bits),
        _ => {
            return Err(crate::SdkError::Unsupported(
                "unknown merge operator".into(),
            ));
        }
    })
}
