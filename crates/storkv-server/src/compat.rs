//! Drop-in wire compatibility front ends: Redis RESP, Memcached ASCII, and the
//! etcd v3 JSON gateway.
//!
//! Every front end maps onto ordinary StorKV scopes, so data written through a
//! compatibility listener is visible to native clients, participates in MVCC,
//! and is covered by autonomous backups.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use storkv_core::{CompactionStyle, Db, Retention, ScopeHandle, ScopeOptions};
use tokio::net::TcpListener;

use crate::server::Server;

pub mod etcd_http;
pub mod memcached;
pub mod redis;

pub(crate) const REDIS_DEFAULT_SCOPE: &str = "default";
pub(crate) const MEMCACHED_SCOPE: &str = "memcached";
pub(crate) const ETCD_SCOPE: &str = "etcd";

/// Redis logical database name. `SELECT 0` maps to the default scope.
pub(crate) fn redis_scope_name(database: u32) -> String {
    if database == 0 {
        REDIS_DEFAULT_SCOPE.to_string()
    } else {
        format!("redis_db_{database}")
    }
}

/// Open a compatibility scope, creating it with strict durability when absent.
///
/// A replica must not create scopes implicitly: reads and writes on a replica
/// only see scopes that already exist.
pub(crate) fn ensure_scope(
    db: &Db,
    allow_create: bool,
    name: &str,
    options: ScopeOptions,
) -> storkv_core::Result<ScopeHandle> {
    if let Some(scope_id) = db.inner().scope_names.read().get(name).copied() {
        return Ok(ScopeHandle {
            scope_id,
            name: name.to_string(),
        });
    }
    if !allow_create {
        return Err(storkv_core::StorkvError::ScopeNotFound(name.to_string()));
    }
    match db.create_scope(name.to_string(), options) {
        Ok(scope) => Ok(scope),
        Err(storkv_core::StorkvError::ScopeAlreadyExists(_)) => {
            let scope_id = db
                .inner()
                .scope_names
                .read()
                .get(name)
                .copied()
                .ok_or_else(|| storkv_core::StorkvError::ScopeNotFound(name.to_string()))?;
            Ok(ScopeHandle {
                scope_id,
                name: name.to_string(),
            })
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn redis_scope(
    db: &Db,
    allow_create: bool,
    database: u32,
) -> storkv_core::Result<ScopeHandle> {
    let name = redis_scope_name(database);
    ensure_scope(
        db,
        allow_create,
        &name,
        ScopeOptions {
            durability: storkv_core::Durability::StrictFsync,
            retention: Retention::Permanent,
            prefix_bloom_len: Some(8),
            storage_mode: storkv_core::StorageMode::Disk,
            ..ScopeOptions::default()
        },
    )
}

pub(crate) fn memcached_scope(db: &Db, allow_create: bool) -> storkv_core::Result<ScopeHandle> {
    ensure_scope(
        db,
        allow_create,
        MEMCACHED_SCOPE,
        ScopeOptions {
            durability: storkv_core::Durability::StrictFsync,
            retention: Retention::Ttl {
                default_ttl: Duration::ZERO,
                compaction_style: CompactionStyle::Fifo,
            },
            prefix_bloom_len: Some(8),
            storage_mode: storkv_core::StorageMode::Disk,
            ..ScopeOptions::default()
        },
    )
}

pub(crate) fn etcd_scope(db: &Db, allow_create: bool) -> storkv_core::Result<ScopeHandle> {
    ensure_scope(
        db,
        allow_create,
        ETCD_SCOPE,
        ScopeOptions {
            durability: storkv_core::Durability::StrictFsync,
            retention: Retention::Permanent,
            prefix_bloom_len: Some(8),
            storage_mode: storkv_core::StorageMode::Disk,
            ..ScopeOptions::default()
        },
    )
}

/// Remove every key in a scope while keeping the scope itself.
///
/// The method writes one range tombstone. It does not load the keys or the
/// values into memory.
///
/// # Errors
///
/// Returns an error when the range delete fails.
pub(crate) fn flush_scope(db: &Db, scope: &ScopeHandle) -> storkv_core::Result<()> {
    let start: &[u8] = b"";
    let end_key = vec![0xFFu8; storkv_core::format::MAX_KEY_LEN];
    let mut transaction = db.begin_tx();
    transaction.delete_range(scope, start, end_key.as_slice())?;
    transaction.delete(scope, end_key.as_slice())?;
    transaction.commit()
}

/// Flush every Redis-owned scope (used by `FLUSHALL`).
///
/// # Errors
///
/// Returns an error when a scope flush fails.
pub(crate) fn flush_redis_scopes(db: &Db) -> storkv_core::Result<()> {
    let names: Vec<String> = db
        .inner()
        .scope_names
        .read()
        .keys()
        .filter(|name| *name == REDIS_DEFAULT_SCOPE || name.starts_with("redis_db_"))
        .cloned()
        .collect();
    for name in names {
        if let Some(scope_id) = db.inner().scope_names.read().get(&name).copied() {
            let scope = ScopeHandle {
                scope_id,
                name: name.clone(),
            };
            flush_scope(db, &scope)?;
        }
    }
    Ok(())
}

/// Serve the Redis RESP listener until the server shuts down.
///
/// # Errors
///
/// Returns an I/O error when the accept loop fails.
pub async fn serve_redis_tcp(server: Arc<Server>, listener: TcpListener) -> std::io::Result<()> {
    redis::serve(server, listener).await
}

/// Serve the Memcached ASCII listener until the server shuts down.
///
/// # Errors
///
/// Returns an I/O error when the accept loop fails.
pub async fn serve_memcached_tcp(
    server: Arc<Server>,
    listener: TcpListener,
) -> std::io::Result<()> {
    memcached::serve(server, listener).await
}

/// Storage envelope used by the Memcached adapter: flags followed by the value.
pub(crate) fn encode_memcached_value(flags: u32, value: &[u8]) -> Bytes {
    let mut buffer = Vec::with_capacity(4 + value.len());
    buffer.extend_from_slice(&flags.to_le_bytes());
    buffer.extend_from_slice(value);
    Bytes::from(buffer)
}

pub(crate) fn decode_memcached_value(stored: &[u8]) -> (u32, &[u8]) {
    if stored.len() < 4 {
        return (0, stored);
    }
    let flags = u32::from_le_bytes([stored[0], stored[1], stored[2], stored[3]]);
    (flags, &stored[4..])
}
