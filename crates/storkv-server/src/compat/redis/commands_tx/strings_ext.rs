//! Extended string commands inside a Redis transaction.

use bytes::Bytes;
use storkv_core::{ScopeHandle, Transaction};

use super::super::codec::Resp;
use super::super::helpers::{GetExMode, parse_getex_options, storage_error, wrong_arity};

pub(super) fn tx_getdel(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() != 2 {
        return wrong_arity("getdel");
    }
    let Some(key) = args.get(1) else {
        return wrong_arity("getdel");
    };
    match tx.get(scope, key) {
        Ok(Some(value)) => match tx.delete(scope, key) {
            Ok(()) => Resp::bulk(value),
            Err(error) => storage_error(error),
        },
        Ok(None) => Resp::null(),
        Err(error) => storage_error(error),
    }
}

pub(super) fn tx_getex(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() < 2 {
        return wrong_arity("getex");
    }
    let Some(key) = args.get(1) else {
        return wrong_arity("getex");
    };
    let mode = match parse_getex_options(args) {
        Ok(mode) => mode,
        Err(response) => return response,
    };
    let value = match tx.get(scope, key) {
        Ok(Some(value)) => value,
        Ok(None) => return Resp::null(),
        Err(error) => return storage_error(error),
    };
    let expires = match mode {
        None => return Resp::bulk(value),
        Some(GetExMode::Persist) => 0,
        Some(GetExMode::ExpireAt(expires_at_ms)) => expires_at_ms,
    };
    match tx.put_with_expires_at(scope, key, &value, expires) {
        Ok(()) => Resp::bulk(value),
        Err(error) => storage_error(error),
    }
}

pub(super) fn tx_msetnx(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() < 3 || !(args.len() - 1).is_multiple_of(2) {
        return wrong_arity("msetnx");
    }
    let mut index = 1;
    while index + 1 < args.len() {
        match tx.get(scope, &args[index]) {
            Ok(Some(_)) => return Resp::Integer(0),
            Ok(None) => {}
            Err(error) => return storage_error(error),
        }
        index += 2;
    }
    let mut index = 1;
    while index + 1 < args.len() {
        if let Err(error) = tx.put_with_expires_at(scope, &args[index], &args[index + 1], 0) {
            return storage_error(error);
        }
        index += 2;
    }
    Resp::Integer(1)
}
