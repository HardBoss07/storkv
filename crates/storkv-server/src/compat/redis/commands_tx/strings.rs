//! String and key commands inside a Redis transaction.

use bytes::Bytes;
use storkv_core::{ScopeHandle, Transaction};

use super::super::codec::Resp;
use super::super::commands::RedisSession;
use super::super::helpers::{parse_set_options, storage_error, wrong_arity};
use super::super::keyspace;

pub(super) fn tx_get(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    let Some(key) = args.get(1) else {
        return wrong_arity("get");
    };
    match tx.get(scope, key) {
        Ok(Some(value)) => Resp::bulk(value),
        Ok(None) => Resp::null(),
        Err(error) => storage_error(error),
    }
}

pub(super) fn tx_set(
    session: &RedisSession,
    tx: &mut Transaction,
    scope: &ScopeHandle,
    args: &[Bytes],
) -> Resp {
    let (Some(key), Some(value)) = (args.get(1), args.get(2)) else {
        return wrong_arity("set");
    };
    let options = match parse_set_options(args) {
        Ok(options) => options,
        Err(response) => return response,
    };
    let previous = match tx.get(scope, key) {
        Ok(previous) => previous,
        Err(error) => return storage_error(error),
    };
    if options.only_if_absent && previous.is_some() {
        return set_reply(options.return_previous, previous, false);
    }
    if options.only_if_present && previous.is_none() {
        return set_reply(options.return_previous, None, false);
    }
    let expires = if options.keep_ttl {
        match session.db().key_meta(scope, key) {
            Ok(meta) => meta.map(|meta| meta.expires_at_ms).unwrap_or(0),
            Err(error) => return storage_error(error),
        }
    } else {
        keyspace::expiry_from_ttl(options.ttl)
    };
    if let Err(error) = tx.put_with_expires_at(scope, key, value, expires) {
        return storage_error(error);
    }
    set_reply(options.return_previous, previous, true)
}

fn set_reply(return_previous: bool, previous: Option<Bytes>, written: bool) -> Resp {
    if return_previous {
        match previous {
            Some(value) => Resp::bulk(value),
            None => Resp::null(),
        }
    } else if written {
        Resp::simple("OK")
    } else {
        Resp::null()
    }
}

pub(super) fn tx_setex(
    tx: &mut Transaction,
    scope: &ScopeHandle,
    args: &[Bytes],
    scale: u64,
) -> Resp {
    let (Some(key), Some(ttl), Some(value)) = (args.get(1), args.get(2), args.get(3)) else {
        return wrong_arity("setex");
    };
    let Some(ttl) = super::super::helpers::parse_i64(ttl) else {
        return Resp::error("ERR invalid expire time");
    };
    if ttl <= 0 {
        return Resp::error("ERR invalid expire time");
    }
    let ttl = std::time::Duration::from_millis((ttl as u64).saturating_mul(scale));
    match tx.put_with_expires_at(scope, key, value, keyspace::expiry_from_ttl(Some(ttl))) {
        Ok(()) => Resp::simple("OK"),
        Err(error) => storage_error(error),
    }
}

pub(super) fn tx_setnx(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    let (Some(key), Some(value)) = (args.get(1), args.get(2)) else {
        return wrong_arity("setnx");
    };
    match tx.get(scope, key) {
        Ok(Some(_)) => Resp::Integer(0),
        Ok(None) => match tx.put_with_expires_at(scope, key, value, 0) {
            Ok(()) => Resp::Integer(1),
            Err(error) => storage_error(error),
        },
        Err(error) => storage_error(error),
    }
}

pub(super) fn tx_mget(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() < 2 {
        return wrong_arity("mget");
    }
    let mut values = Vec::with_capacity(args.len() - 1);
    for key in &args[1..] {
        match tx.get(scope, key) {
            Ok(Some(value)) => values.push(Resp::bulk(value)),
            Ok(None) => values.push(Resp::null()),
            Err(error) => return storage_error(error),
        }
    }
    Resp::array(values)
}

pub(super) fn tx_mset(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() < 3 || (args.len() - 1) % 2 != 0 {
        return wrong_arity("mset");
    }
    let mut index = 1;
    while index + 1 < args.len() {
        if let Err(error) = tx.put_with_expires_at(scope, &args[index], &args[index + 1], 0) {
            return storage_error(error);
        }
        index += 2;
    }
    Resp::simple("OK")
}

pub(super) fn tx_delete(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() < 2 {
        return wrong_arity("del");
    }
    let mut removed = 0;
    for key in &args[1..] {
        match tx.get(scope, key) {
            Ok(Some(_)) => {
                if let Err(error) = tx.delete(scope, key) {
                    return storage_error(error);
                }
                removed += 1;
            }
            Ok(None) => {}
            Err(error) => return storage_error(error),
        }
    }
    Resp::Integer(removed)
}

pub(super) fn tx_exists(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() < 2 {
        return wrong_arity("exists");
    }
    let mut count = 0;
    for key in &args[1..] {
        match tx.get(scope, key) {
            Ok(Some(_)) => count += 1,
            Ok(None) => {}
            Err(error) => return storage_error(error),
        }
    }
    Resp::Integer(count)
}

pub(super) fn tx_append(
    session: &RedisSession,
    tx: &mut Transaction,
    scope: &ScopeHandle,
    args: &[Bytes],
) -> Resp {
    let (Some(key), Some(suffix)) = (args.get(1), args.get(2)) else {
        return wrong_arity("append");
    };
    let current = match tx.get(scope, key) {
        Ok(current) => current.unwrap_or_default(),
        Err(error) => return storage_error(error),
    };
    let mut combined = Vec::with_capacity(current.len() + suffix.len());
    combined.extend_from_slice(&current);
    combined.extend_from_slice(suffix);
    let expires = if current.is_empty() {
        0
    } else {
        match session.db().key_meta(scope, key) {
            Ok(meta) => meta.map(|meta| meta.expires_at_ms).unwrap_or(0),
            Err(error) => return storage_error(error),
        }
    };
    match tx.put_with_expires_at(scope, key, &combined, expires) {
        Ok(()) => Resp::Integer(combined.len() as i64),
        Err(error) => storage_error(error),
    }
}

pub(super) fn tx_strlen(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    let Some(key) = args.get(1) else {
        return wrong_arity("strlen");
    };
    match tx.get(scope, key) {
        Ok(Some(value)) => Resp::Integer(value.len() as i64),
        Ok(None) => Resp::Integer(0),
        Err(error) => storage_error(error),
    }
}
