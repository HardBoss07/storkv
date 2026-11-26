//! Hash and set commands inside a Redis transaction.

use bytes::Bytes;
use storkv_core::{ScopeHandle, StorkvError, Transaction};

use super::super::codec::Resp;
use super::super::composite;
use super::super::helpers::storage_error;
use super::super::keyspace::Outcome;

pub(super) fn tx_hset(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    finish(composite::hset(tx, scope, args))
}

pub(super) fn tx_hdel(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    finish(composite::hdel(tx, scope, args))
}

pub(super) fn tx_hincrby(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    finish(composite::hincrby(tx, scope, args))
}

pub(super) fn tx_sadd(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    finish(composite::sadd(tx, scope, args))
}

pub(super) fn tx_srem(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    finish(composite::srem(tx, scope, args))
}

pub(super) fn tx_hget(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    composite::hget(tx, scope, args)
}

pub(super) fn tx_hmget(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    composite::hmget(tx, scope, args)
}

pub(super) fn tx_hgetall(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    composite::hgetall(tx, scope, args)
}

pub(super) fn tx_hexists(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    composite::hexists(tx, scope, args)
}

pub(super) fn tx_hlen(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    composite::hlen(tx, scope, args)
}

pub(super) fn tx_hkeys(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    composite::hkeys(tx, scope, args)
}

pub(super) fn tx_hvals(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    composite::hvals(tx, scope, args)
}

pub(super) fn tx_smembers(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    composite::smembers(tx, scope, args)
}

pub(super) fn tx_sismember(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    composite::sismember(tx, scope, args)
}

pub(super) fn tx_scard(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    composite::scard(tx, scope, args)
}

fn finish(result: Result<Outcome<Resp>, StorkvError>) -> Resp {
    match result {
        Ok(Outcome::Commit(response)) => response,
        Ok(Outcome::Reply(response)) => response,
        Err(error) => storage_error(error),
    }
}
