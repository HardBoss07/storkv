//! Networked client operations for a checked-out connection.

use bytes::Bytes;
use storkv_client::{AsyncClient, ClientError};
use storkv_proto::{Consistency, StorkvWireError, WireDurability, WireRetention, WireScopeOptions};

use crate::error::{Result, SdkError};
use crate::options::ScopeDefinition;

use super::{NetworkedBackend, ResolvedScope};

pub(super) async fn set_consistency(client: &AsyncClient, consistency: Consistency) -> Result<()> {
    client
        .set_consistency(consistency)
        .await
        .map_err(SdkError::from)
}

pub(super) async fn get(
    client: &AsyncClient,
    scope: &ResolvedScope,
    key: &[u8],
) -> Result<Option<Bytes>> {
    client
        .get(&scope.client_handle(), key)
        .await
        .map_err(SdkError::from)
}

pub(super) async fn key_meta(
    client: &AsyncClient,
    scope: &ResolvedScope,
    key: &[u8],
) -> Result<Option<storkv_proto::WireKeyMeta>> {
    client
        .key_meta(&scope.client_handle(), key)
        .await
        .map_err(SdkError::from)
}

pub(super) async fn put(
    client: &AsyncClient,
    scope: &ResolvedScope,
    key: &[u8],
    value: Bytes,
) -> Result<()> {
    client
        .put(&scope.client_handle(), key, value)
        .await
        .map_err(SdkError::from)
}

pub(super) async fn put_with_ttl(
    client: &AsyncClient,
    scope: &ResolvedScope,
    key: &[u8],
    value: Bytes,
    ttl: std::time::Duration,
) -> Result<()> {
    client
        .put_with_ttl(&scope.client_handle(), key, value, ttl)
        .await
        .map_err(SdkError::from)
}

pub(super) async fn put_if(
    client: &AsyncClient,
    scope: &ResolvedScope,
    key: &[u8],
    condition: &storkv_core::PutCondition,
    value: Bytes,
    ttl: Option<std::time::Duration>,
) -> Result<storkv_core::PutIfResult> {
    let wire_condition = condition_to_wire(condition)?;
    let result = client
        .put_if(&scope.client_handle(), key, &wire_condition, value, ttl)
        .await
        .map_err(SdkError::from)?;
    match result {
        storkv_proto::WirePutIfResult::Applied { seq } => {
            Ok(storkv_core::PutIfResult::Applied { seq })
        }
        storkv_proto::WirePutIfResult::Conflict {
            current_seq,
            current_value,
        } => Ok(storkv_core::PutIfResult::Conflict {
            current_seq,
            current_value,
        }),
        _ => Err(SdkError::Unsupported("unknown put-if result".into())),
    }
}

fn condition_to_wire(
    condition: &storkv_core::PutCondition,
) -> Result<storkv_proto::WirePutCondition> {
    use storkv_core::PutCondition;
    Ok(match condition {
        PutCondition::Absent => storkv_proto::WirePutCondition::Absent,
        PutCondition::Present => storkv_proto::WirePutCondition::Present,
        PutCondition::ExpectedSeq(seq) => storkv_proto::WirePutCondition::ExpectedSeq(*seq),
        PutCondition::ExpectedValue(value) => {
            storkv_proto::WirePutCondition::ExpectedValue(value.clone())
        }
        _ => return Err(SdkError::Unsupported("unknown put condition".into())),
    })
}

pub(super) async fn delete(client: &AsyncClient, scope: &ResolvedScope, key: &[u8]) -> Result<()> {
    client
        .delete(&scope.client_handle(), key)
        .await
        .map_err(SdkError::from)
}

pub(super) async fn scan_prefix(
    client: &AsyncClient,
    scope: &ResolvedScope,
    prefix: &[u8],
) -> Result<Vec<(Bytes, Bytes)>> {
    client
        .scan_prefix(&scope.client_handle(), prefix)
        .await
        .map_err(SdkError::from)
}

pub(super) async fn scan_range(
    client: &AsyncClient,
    scope: &ResolvedScope,
    start: Option<Bytes>,
    end: Option<Bytes>,
) -> Result<Vec<(Bytes, Bytes)>> {
    client
        .scan(&scope.client_handle(), start, end)
        .await
        .map_err(SdkError::from)
}

pub(super) async fn resolve_scope(
    client: &AsyncClient,
    backend: &NetworkedBackend,
    name: &str,
) -> Result<ResolvedScope> {
    if let Some(scope_id) = backend.scopes.lock().await.get(name).copied() {
        return Ok(ResolvedScope {
            scope_id,
            name: name.to_string(),
        });
    }
    let scope_id = scope_id_from_admin(client, name).await?;
    backend.remember_scope(name, scope_id).await;
    Ok(ResolvedScope {
        scope_id,
        name: name.to_string(),
    })
}

async fn scope_id_from_admin(client: &AsyncClient, name: &str) -> Result<u32> {
    let stats = client.admin_stats().await.map_err(SdkError::from)?;
    stats
        .scopes
        .iter()
        .find(|scope| scope.name == name)
        .map(|scope| scope.scope_id)
        .ok_or_else(|| SdkError::ScopeNotFound(name.to_string()))
}

pub(super) async fn provision(
    client: &AsyncClient,
    backend: &NetworkedBackend,
    definition: &ScopeDefinition,
) -> Result<()> {
    let durability = durability_to_wire(&definition.durability);
    let retention = retention_to_wire(&definition.retention);
    match client
        .create_scope_with_options(
            definition.name.clone(),
            WireScopeOptions {
                durability,
                retention,
                prefix_bloom_len: definition.prefix_bloom_len,
                min_ttl: definition.min_ttl,
                max_ttl: definition.max_ttl,
                allow_permanent: definition.allow_permanent,
            },
        )
        .await
    {
        Ok(handle) => {
            backend
                .remember_scope(&definition.name, handle.scope_id)
                .await;
            Ok(())
        }
        Err(ClientError::Server(StorkvWireError::ScopeAlreadyExists(_))) => {
            let scope_id = scope_id_from_admin(client, &definition.name).await?;
            backend.remember_scope(&definition.name, scope_id).await;
            Ok(())
        }
        Err(error) => Err(SdkError::from(error)),
    }
}

fn durability_to_wire(durability: &storkv_core::Durability) -> WireDurability {
    match durability {
        storkv_core::Durability::StrictFsync => WireDurability::StrictFsync,
        storkv_core::Durability::Buffered { sync_interval } => WireDurability::Buffered {
            sync_interval: *sync_interval,
        },
        _ => WireDurability::StrictFsync,
    }
}

fn retention_to_wire(retention: &storkv_core::Retention) -> WireRetention {
    match retention {
        storkv_core::Retention::Permanent => WireRetention::Permanent,
        storkv_core::Retention::Ttl {
            default_ttl,
            compaction_style,
        } => WireRetention::Ttl {
            default_ttl: *default_ttl,
            compaction_style: match compaction_style {
                storkv_core::CompactionStyle::Leveled => storkv_proto::WireCompactionStyle::Leveled,
                storkv_core::CompactionStyle::Fifo => storkv_proto::WireCompactionStyle::Fifo,
                _ => storkv_proto::WireCompactionStyle::Leveled,
            },
        },
        _ => WireRetention::Permanent,
    }
}
