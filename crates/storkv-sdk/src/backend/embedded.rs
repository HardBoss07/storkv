//! Embedded database operations for a checked-out connection.

use bytes::Bytes;
use storkv_core::Db;

use crate::error::{Result, SdkError, join_error};
use crate::options::ScopeDefinition;

use super::ResolvedScope;

pub(super) async fn get(db: &Db, scope: &ResolvedScope, key: &[u8]) -> Result<Option<Bytes>> {
    let db = db.clone();
    let handle = scope.core_handle();
    let key = Bytes::copy_from_slice(key);
    tokio::task::spawn_blocking(move || db.get(&handle, key))
        .await
        .map_err(join_error)?
        .map_err(SdkError::from)
}

pub(super) async fn key_meta(
    db: &Db,
    scope: &ResolvedScope,
    key: &[u8],
) -> Result<Option<storkv_proto::WireKeyMeta>> {
    let db = db.clone();
    let handle = scope.core_handle();
    let key = Bytes::copy_from_slice(key);
    tokio::task::spawn_blocking(move || db.key_meta(&handle, key))
        .await
        .map_err(join_error)?
        .map(|meta| {
            meta.map(|meta| storkv_proto::WireKeyMeta {
                expires_at_ms: meta.expires_at_ms,
                seq: meta.seq,
            })
        })
        .map_err(SdkError::from)
}

pub(super) async fn put(db: &Db, scope: &ResolvedScope, key: &[u8], value: Bytes) -> Result<()> {
    let db = db.clone();
    let handle = scope.core_handle();
    let key = Bytes::copy_from_slice(key);
    tokio::task::spawn_blocking(move || db.put(&handle, key, value))
        .await
        .map_err(join_error)?
        .map_err(SdkError::from)
}

pub(super) async fn put_with_ttl(
    db: &Db,
    scope: &ResolvedScope,
    key: &[u8],
    value: Bytes,
    ttl: std::time::Duration,
) -> Result<()> {
    let db = db.clone();
    let handle = scope.core_handle();
    let key = Bytes::copy_from_slice(key);
    tokio::task::spawn_blocking(move || db.put_with_ttl(&handle, key, value, ttl))
        .await
        .map_err(join_error)?
        .map_err(SdkError::from)
}

pub(super) async fn put_if(
    db: &Db,
    scope: &ResolvedScope,
    key: &[u8],
    condition: &storkv_core::PutCondition,
    value: Bytes,
    ttl: Option<std::time::Duration>,
) -> Result<storkv_core::PutIfResult> {
    let db = db.clone();
    let handle = scope.core_handle();
    let key = Bytes::copy_from_slice(key);
    let condition = condition.clone();
    tokio::task::spawn_blocking(move || db.put_if(&handle, key, &condition, Some(value), ttl))
        .await
        .map_err(join_error)?
        .map_err(SdkError::from)
}

pub(super) async fn delete(db: &Db, scope: &ResolvedScope, key: &[u8]) -> Result<()> {
    let db = db.clone();
    let handle = scope.core_handle();
    let key = Bytes::copy_from_slice(key);
    tokio::task::spawn_blocking(move || db.delete(&handle, key))
        .await
        .map_err(join_error)?
        .map_err(SdkError::from)
}

pub(super) async fn merge(
    db: &Db,
    scope: &ResolvedScope,
    key: &[u8],
    op: &storkv_core::MergeOp,
) -> Result<()> {
    let db = db.clone();
    let handle = scope.core_handle();
    let key = Bytes::copy_from_slice(key);
    let op = op.clone();
    tokio::task::spawn_blocking(move || db.merge(&handle, key, op))
        .await
        .map_err(join_error)?
        .map_err(SdkError::from)
}

pub(super) async fn scan_prefix(
    db: &Db,
    scope: &ResolvedScope,
    prefix: &[u8],
) -> Result<Vec<(Bytes, Bytes)>> {
    let db = db.clone();
    let handle = scope.core_handle();
    let prefix = Bytes::copy_from_slice(prefix);
    tokio::task::spawn_blocking(move || db.scan_prefix(&handle, prefix))
        .await
        .map_err(join_error)?
        .map_err(SdkError::from)
}

pub(super) async fn scan_range(
    db: &Db,
    scope: &ResolvedScope,
    start: Option<Bytes>,
    end: Option<Bytes>,
) -> Result<Vec<(Bytes, Bytes)>> {
    let db = db.clone();
    let handle = scope.core_handle();
    tokio::task::spawn_blocking(move || db.scan(&handle, start, end))
        .await
        .map_err(join_error)?
        .map_err(SdkError::from)
}

pub(super) async fn resolve_scope(db: &Db, name: &str) -> Result<ResolvedScope> {
    let db = db.clone();
    let name = name.to_string();
    let expected = name.clone();
    tokio::task::spawn_blocking(move || {
        db.inner()
            .scope_names
            .read()
            .get(&name)
            .copied()
            .map(|scope_id| ResolvedScope {
                scope_id,
                name: name.clone(),
            })
    })
    .await
    .map_err(join_error)?
    .ok_or_else(|| SdkError::ScopeNotFound(expected))
}

pub(super) async fn provision(db: &Db, definition: &ScopeDefinition) -> Result<()> {
    let db = db.clone();
    let definition = definition.clone();
    tokio::task::spawn_blocking(move || {
        let options = storkv_core::ScopeOptions {
            durability: definition.durability,
            retention: definition.retention,
            prefix_bloom_len: definition.prefix_bloom_len,
            storage_mode: storkv_core::StorageMode::Disk,
            min_ttl: definition.min_ttl,
            max_ttl: definition.max_ttl,
            allow_permanent: definition.allow_permanent,
            active_ttl_tracking: false,
        };
        match db.create_scope(definition.name.clone(), options) {
            Ok(_) | Err(storkv_core::StorkvError::ScopeAlreadyExists(_)) => Ok(()),
            Err(error) => Err(error),
        }
    })
    .await
    .map_err(join_error)?
    .map_err(SdkError::from)
}
