//! Key, type, and absolute expiry commands inside a Redis transaction.

use bytes::Bytes;
use storkv_core::{ScopeHandle, TimestampMs, Transaction};

use super::super::codec::Resp;
use super::super::commands::MILLIS_PER_SECOND;
use super::super::helpers::{parse_expire_conditions, parse_i64, storage_error, wrong_arity};

pub(super) fn tx_rename(
    tx: &mut Transaction,
    scope: &ScopeHandle,
    args: &[Bytes],
    only_if_absent: bool,
) -> Resp {
    let command = if only_if_absent { "renamenx" } else { "rename" };
    let (Some(key), Some(new_key)) = (args.get(1), args.get(2)) else {
        return wrong_arity(command);
    };
    if args.len() != 3 {
        return wrong_arity(command);
    }
    let value = match tx.get(scope, key) {
        Ok(Some(value)) => value,
        Ok(None) => return Resp::error("ERR no such key"),
        Err(error) => return storage_error(error),
    };
    if key == new_key {
        return if only_if_absent {
            Resp::Integer(0)
        } else {
            Resp::simple("OK")
        };
    }
    if only_if_absent {
        match tx.get(scope, new_key) {
            Ok(Some(_)) => return Resp::Integer(0),
            Ok(None) => {}
            Err(error) => return storage_error(error),
        }
    }
    let expires = match tx.key_meta(scope, key) {
        Ok(meta) => meta.map(|meta| meta.expires_at_ms).unwrap_or(0),
        Err(error) => return storage_error(error),
    };
    if let Err(error) = tx.put_with_expires_at(scope, new_key, &value, expires) {
        return storage_error(error);
    }
    if let Err(error) = tx.delete(scope, key) {
        return storage_error(error);
    }
    if only_if_absent {
        Resp::Integer(1)
    } else {
        Resp::simple("OK")
    }
}

pub(super) fn tx_type(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() != 2 {
        return wrong_arity("type");
    }
    let Some(key) = args.get(1) else {
        return wrong_arity("type");
    };
    match super::super::composite::read_kind(tx, scope, key) {
        Ok(Some(kind)) => return Resp::simple(kind.name()),
        Ok(None) => {}
        Err(error) => return storage_error(error),
    }
    match tx.get(scope, key) {
        Ok(Some(_)) => Resp::simple("string"),
        Ok(None) => Resp::simple("none"),
        Err(error) => storage_error(error),
    }
}

pub(super) fn tx_expire_at(
    tx: &mut Transaction,
    scope: &ScopeHandle,
    args: &[Bytes],
    scale: u64,
) -> Resp {
    let command = if scale == MILLIS_PER_SECOND {
        "expireat"
    } else {
        "pexpireat"
    };
    let (Some(key), Some(value)) = (args.get(1), args.get(2)) else {
        return wrong_arity(command);
    };
    let Some(value) = parse_i64(value) else {
        return Resp::error("ERR value is not an integer or out of range");
    };
    let conditions = match parse_expire_conditions(args, 3) {
        Ok(conditions) => conditions,
        Err(response) => return response,
    };
    let stored = match tx.get(scope, key) {
        Ok(Some(value)) => value,
        Ok(None) => return Resp::Integer(0),
        Err(error) => return storage_error(error),
    };
    let current = match tx.key_meta(scope, key) {
        Ok(meta) => meta.map(|meta| meta.expires_at_ms).unwrap_or(0),
        Err(error) => return storage_error(error),
    };
    let target = (value.max(0) as u64).saturating_mul(scale);
    if !conditions.matches(current, target) {
        return Resp::Integer(0);
    }
    let result = if target <= TimestampMs::now().get() {
        tx.delete(scope, key)
    } else {
        tx.put_with_expires_at(scope, key, &stored, target)
    };
    match result {
        Ok(()) => Resp::Integer(1),
        Err(error) => storage_error(error),
    }
}

pub(super) fn tx_expiretime(
    tx: &mut Transaction,
    scope: &ScopeHandle,
    args: &[Bytes],
    scale: u64,
) -> Resp {
    let command = if scale == MILLIS_PER_SECOND {
        "expiretime"
    } else {
        "pexpiretime"
    };
    if args.len() != 2 {
        return wrong_arity(command);
    }
    let Some(key) = args.get(1) else {
        return wrong_arity(command);
    };
    match tx.key_meta(scope, key) {
        Ok(Some(meta)) if meta.expires_at_ms == 0 => Resp::Integer(-1),
        Ok(Some(meta)) => Resp::Integer((meta.expires_at_ms / scale) as i64),
        Ok(None) => Resp::Integer(-2),
        Err(error) => storage_error(error),
    }
}
