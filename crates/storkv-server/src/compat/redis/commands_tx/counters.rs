//! Counter commands inside a Redis transaction.

use bytes::Bytes;
use storkv_core::{ScopeHandle, Transaction};

use super::super::codec::Resp;
use super::super::commands::RedisSession;
use super::super::helpers::{parse_i64, storage_error, wrong_arity};

pub(super) fn tx_counter(
    session: &RedisSession,
    tx: &mut Transaction,
    scope: &ScopeHandle,
    args: &[Bytes],
    delta: i64,
) -> Resp {
    tx_counter_delta(session, tx, scope, args, delta)
}

pub(super) fn tx_counter_by(
    session: &RedisSession,
    tx: &mut Transaction,
    scope: &ScopeHandle,
    args: &[Bytes],
    sign: i64,
) -> Resp {
    let Some(delta) = args.get(2).and_then(|arg| parse_i64(arg)) else {
        return Resp::error("ERR value is not an integer or out of range");
    };
    let Some(delta) = delta.checked_mul(sign) else {
        return Resp::error("ERR increment or decrement would overflow");
    };
    tx_counter_delta(session, tx, scope, args, delta)
}

fn tx_counter_delta(
    session: &RedisSession,
    tx: &mut Transaction,
    scope: &ScopeHandle,
    args: &[Bytes],
    delta: i64,
) -> Resp {
    let Some(key) = args.get(1) else {
        return wrong_arity("incr");
    };
    let current = match tx.get(scope, key) {
        Ok(current) => current,
        Err(error) => return storage_error(error),
    };
    let base = match current.as_deref() {
        Some(value) => match std::str::from_utf8(value)
            .ok()
            .and_then(|text| text.parse::<i64>().ok())
        {
            Some(value) => value,
            None => return Resp::error("ERR value is not an integer or out of range"),
        },
        None => 0,
    };
    let Some(next) = base.checked_add(delta) else {
        return Resp::error("ERR increment or decrement would overflow");
    };
    let expires = if current.is_some() {
        match session.db().key_meta(scope, key) {
            Ok(meta) => meta.map(|meta| meta.expires_at_ms).unwrap_or(0),
            Err(error) => return storage_error(error),
        }
    } else {
        0
    };
    match tx.put_with_expires_at(scope, key, next.to_string().as_bytes(), expires) {
        Ok(()) => Resp::Integer(next),
        Err(error) => storage_error(error),
    }
}
