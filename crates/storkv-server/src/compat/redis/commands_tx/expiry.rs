//! Expiry commands inside a Redis transaction.

use bytes::Bytes;
use storkv_core::{ScopeHandle, Transaction};

use super::super::codec::Resp;
use super::super::commands::RedisSession;
use super::super::helpers::{parse_i64, storage_error, wrong_arity};
use super::super::keyspace;

pub(super) fn tx_expire(
    tx: &mut Transaction,
    scope: &ScopeHandle,
    args: &[Bytes],
    scale: u64,
) -> Resp {
    let (Some(key), Some(ttl)) = (args.get(1), args.get(2)) else {
        return wrong_arity("expire");
    };
    let Some(ttl) = parse_i64(ttl) else {
        return Resp::error("ERR value is not an integer or out of range");
    };
    let value = match tx.get(scope, key) {
        Ok(Some(value)) => value,
        Ok(None) => return Resp::Integer(0),
        Err(error) => return storage_error(error),
    };
    if ttl <= 0 {
        return match tx.delete(scope, key) {
            Ok(()) => Resp::Integer(1),
            Err(error) => storage_error(error),
        };
    }
    let ttl = std::time::Duration::from_millis((ttl as u64).saturating_mul(scale));
    match tx.put_with_expires_at(scope, key, &value, keyspace::expiry_from_ttl(Some(ttl))) {
        Ok(()) => Resp::Integer(1),
        Err(error) => storage_error(error),
    }
}

pub(super) fn tx_persist(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    let Some(key) = args.get(1) else {
        return wrong_arity("persist");
    };
    let value = match tx.get(scope, key) {
        Ok(Some(value)) => value,
        Ok(None) => return Resp::Integer(0),
        Err(error) => return storage_error(error),
    };
    match tx.put_with_expires_at(scope, key, &value, 0) {
        Ok(()) => Resp::Integer(1),
        Err(error) => storage_error(error),
    }
}

pub(super) fn tx_ttl(
    session: &RedisSession,
    scope: &ScopeHandle,
    args: &[Bytes],
    scale: u64,
) -> Resp {
    let Some(key) = args.get(1) else {
        return wrong_arity("ttl");
    };
    match keyspace::remaining_ms(session.db(), scope, key) {
        Ok(None) => Resp::Integer(-2),
        Ok(Some(0)) => Resp::Integer(-1),
        Ok(Some(remaining)) if scale == 1000 => Resp::Integer(remaining.div_ceil(1000) as i64),
        Ok(Some(remaining)) => Resp::Integer(remaining as i64),
        Err(error) => storage_error(error),
    }
}
