use std::error::Error;
use std::time::Duration;

use bytes::Bytes;
use storkv_client::{AsyncClient, ScopeHandle};
use storkv_core::mvcc::TransactionContext;
use storkv_core::{Db, DbOptions, StorkvError};
use storkv_proto::{WireDurability, WireRetention};

use super::{MigrateTarget, Record, decode};

pub(super) async fn apply_records(
    target: MigrateTarget<'_>,
    scope_name: &str,
    records: &[Record],
) -> Result<usize, Box<dyn Error>> {
    match target {
        MigrateTarget::DataDir(directory) => {
            apply_offline(directory, scope_name, records)?;
            Ok(records.len())
        }
        MigrateTarget::Daemon(socket) => {
            apply_online(socket, scope_name, records).await?;
            Ok(records.len())
        }
    }
}

fn apply_offline(
    directory: &std::path::Path,
    scope_name: &str,
    records: &[Record],
) -> Result<(), Box<dyn Error>> {
    let db = Db::open(directory, DbOptions::default())?;
    let scope = open_offline_scope(&db, scope_name)?;
    let read_seq = db
        .inner()
        .last_visible_seq
        .load(std::sync::atomic::Ordering::Acquire);
    let mut transaction = TransactionContext::new(read_seq);
    for (key, value, expires_at_ms) in decoded(records) {
        transaction.put(scope.scope_id, key, value, expires_at_ms);
    }
    db.inner().commit_tx(transaction)?;
    Ok(())
}

async fn apply_online(
    socket: &str,
    scope_name: &str,
    records: &[Record],
) -> Result<(), Box<dyn Error>> {
    let client = AsyncClient::connect_uri(socket).await?;
    let scope = open_online_scope(&client, scope_name).await?;
    let now_ms = storkv_core::TimestampMs::now().get();
    for (key, value, expires_at_ms) in decoded(records) {
        if expires_at_ms == 0 {
            client.put(&scope, &key, &value).await?;
        } else {
            let remaining = expires_at_ms.saturating_sub(now_ms).max(1);
            client
                .put_with_ttl(&scope, &key, &value, Duration::from_millis(remaining))
                .await?;
        }
    }
    Ok(())
}

fn decoded(records: &[Record]) -> impl Iterator<Item = (Bytes, Bytes, u64)> + '_ {
    records.iter().filter_map(|record| {
        let key = decode(&record.key_b64)?;
        let value = decode(&record.value_b64)?;
        Some((Bytes::from(key), Bytes::from(value), record.expires_at_ms))
    })
}

/// Resolve a scope by name, creating it when it does not exist.
pub(super) async fn open_online_scope(
    client: &AsyncClient,
    name: &str,
) -> Result<ScopeHandle, Box<dyn Error>> {
    if name == "default" {
        return Ok(ScopeHandle {
            scope_id: 0,
            name: name.to_string(),
        });
    }
    if let Ok(stats) = client.admin_stats().await {
        if let Some(scope) = stats.scopes.iter().find(|scope| scope.name == name) {
            return Ok(ScopeHandle {
                scope_id: scope.scope_id,
                name: scope.name.clone(),
            });
        }
    }
    let scope = client
        .create_scope(
            name,
            WireDurability::StrictFsync,
            WireRetention::Permanent,
            Some(8),
        )
        .await?;
    Ok(scope)
}

fn open_offline_scope(db: &Db, name: &str) -> Result<storkv_core::ScopeHandle, StorkvError> {
    if let Some(scope_id) = db.inner().scope_names.read().get(name).copied() {
        return Ok(storkv_core::ScopeHandle {
            scope_id,
            name: name.to_string(),
        });
    }
    db.create_scope(
        name,
        storkv_core::ScopeOptions {
            durability: storkv_core::Durability::StrictFsync,
            retention: storkv_core::Retention::Permanent,
            prefix_bloom_len: Some(8),
            storage_mode: storkv_core::StorageMode::Disk,
            ..storkv_core::ScopeOptions::default()
        },
    )
}
