//! Generic hash command logic shared by direct and transactional commands.

use bytes::Bytes;
use storkv_core::{ScopeHandle, StorkvError};

use super::super::codec::Resp;
use super::super::helpers::{parse_i64, storage_error, wrong_arity};
use super::super::keyspace::Outcome;
use super::keys::{self, Kind, Lookup};
use super::view::{ReadView, WriteView};

mod scan;

pub(crate) use scan::{hgetall, hkeys, hvals};

/// Run `HSET`.
pub(crate) fn hset<V: WriteView>(
    view: &mut V,
    scope: &ScopeHandle,
    args: &[Bytes],
) -> Result<Outcome<Resp>, StorkvError> {
    if args.len() < 4 || !(args.len() - 2).is_multiple_of(2) {
        return Ok(Outcome::Reply(wrong_arity("hset")));
    }
    let key = &args[1];
    let (mut count, existed) = match keys::lookup(view, scope, key, Kind::Hash)? {
        Lookup::Absent => (0, false),
        Lookup::Meta(meta) => (meta.count, true),
        Lookup::WrongType => return Ok(Outcome::Reply(keys::wrong_type())),
    };
    let mut added = 0u64;
    let mut index = 2;
    while index + 1 < args.len() {
        let field_key = keys::data_key(Kind::Hash, key, &args[index]);
        if view.get(scope, &field_key)?.is_none() {
            added += 1;
        }
        view.put(scope, &field_key, &args[index + 1], 0)?;
        index += 2;
    }
    if added > 0 || !existed {
        count = count.saturating_add(added);
        keys::write_meta(view, scope, key, Kind::Hash, count)?;
    }
    Ok(Outcome::Commit(Resp::Integer(added as i64)))
}

/// Run `HDEL`.
pub(crate) fn hdel<V: WriteView>(
    view: &mut V,
    scope: &ScopeHandle,
    args: &[Bytes],
) -> Result<Outcome<Resp>, StorkvError> {
    if args.len() < 3 {
        return Ok(Outcome::Reply(wrong_arity("hdel")));
    }
    let key = &args[1];
    let meta = match keys::lookup(view, scope, key, Kind::Hash)? {
        Lookup::Absent => return Ok(Outcome::Commit(Resp::Integer(0))),
        Lookup::Meta(meta) => meta,
        Lookup::WrongType => return Ok(Outcome::Reply(keys::wrong_type())),
    };
    let mut removed = 0u64;
    for field in &args[2..] {
        let field_key = keys::data_key(Kind::Hash, key, field);
        if view.get(scope, &field_key)?.is_some() {
            view.delete(scope, &field_key)?;
            removed += 1;
        }
    }
    if removed > 0 {
        let remaining = meta.count.saturating_sub(removed);
        if remaining == 0 {
            keys::delete_meta(view, scope, key)?;
        } else {
            keys::write_meta(view, scope, key, Kind::Hash, remaining)?;
        }
    }
    Ok(Outcome::Commit(Resp::Integer(removed as i64)))
}

/// Run `HINCRBY`.
pub(crate) fn hincrby<V: WriteView>(
    view: &mut V,
    scope: &ScopeHandle,
    args: &[Bytes],
) -> Result<Outcome<Resp>, StorkvError> {
    if args.len() != 4 {
        return Ok(Outcome::Reply(wrong_arity("hincrby")));
    }
    let Some(delta) = parse_i64(&args[3]) else {
        return Ok(Outcome::Reply(Resp::error(
            "ERR value is not an integer or out of range",
        )));
    };
    let key = &args[1];
    let meta = match keys::lookup(view, scope, key, Kind::Hash)? {
        Lookup::Absent => None,
        Lookup::Meta(meta) => Some(meta.count),
        Lookup::WrongType => return Ok(Outcome::Reply(keys::wrong_type())),
    };
    let field_key = keys::data_key(Kind::Hash, key, &args[2]);
    let current = match view.get(scope, &field_key)? {
        Some(value) => match parse_i64(&value) {
            Some(current) => current,
            None => {
                return Ok(Outcome::Reply(Resp::error(
                    "ERR hash value is not an integer",
                )));
            }
        },
        None => 0,
    };
    let Some(next) = current.checked_add(delta) else {
        return Ok(Outcome::Reply(Resp::error(
            "ERR increment or decrement would overflow",
        )));
    };
    view.put(scope, &field_key, next.to_string().as_bytes(), 0)?;
    if meta.is_none() {
        keys::write_meta(view, scope, key, Kind::Hash, 1)?;
    }
    Ok(Outcome::Commit(Resp::Integer(next)))
}

/// Run `HGET`.
pub(crate) fn hget<V: ReadView>(view: &mut V, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() != 3 {
        return wrong_arity("hget");
    }
    let key = &args[1];
    match keys::lookup(view, scope, key, Kind::Hash) {
        Err(error) => storage_error(error),
        Ok(Lookup::WrongType) => keys::wrong_type(),
        Ok(Lookup::Absent) => Resp::null(),
        Ok(Lookup::Meta(_)) => {
            let field_key = keys::data_key(Kind::Hash, key, &args[2]);
            match view.get(scope, &field_key) {
                Ok(value) => value.map_or_else(Resp::null, Resp::bulk),
                Err(error) => storage_error(error),
            }
        }
    }
}

/// Run `HMGET`.
pub(crate) fn hmget<V: ReadView>(view: &mut V, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() < 3 {
        return wrong_arity("hmget");
    }
    let key = &args[1];
    let present = match keys::lookup(view, scope, key, Kind::Hash) {
        Ok(Lookup::Meta(_)) => true,
        Ok(Lookup::Absent) => false,
        Ok(Lookup::WrongType) => return keys::wrong_type(),
        Err(error) => return storage_error(error),
    };
    let mut values = Vec::with_capacity(args.len() - 2);
    for field in &args[2..] {
        if !present {
            values.push(Resp::null());
            continue;
        }
        let field_key = keys::data_key(Kind::Hash, key, field);
        match view.get(scope, &field_key) {
            Ok(Some(value)) => values.push(Resp::bulk(value)),
            Ok(None) => values.push(Resp::null()),
            Err(error) => return storage_error(error),
        }
    }
    Resp::array(values)
}

/// Run `HEXISTS`.
pub(crate) fn hexists<V: ReadView>(view: &mut V, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() != 3 {
        return wrong_arity("hexists");
    }
    match keys::lookup(view, scope, &args[1], Kind::Hash) {
        Err(error) => storage_error(error),
        Ok(Lookup::WrongType) => keys::wrong_type(),
        Ok(Lookup::Absent) => Resp::Integer(0),
        Ok(Lookup::Meta(_)) => {
            let field_key = keys::data_key(Kind::Hash, &args[1], &args[2]);
            match view.get(scope, &field_key) {
                Ok(Some(_)) => Resp::Integer(1),
                Ok(None) => Resp::Integer(0),
                Err(error) => storage_error(error),
            }
        }
    }
}

/// Run `HLEN`.
pub(crate) fn hlen<V: ReadView>(view: &mut V, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() != 2 {
        return wrong_arity("hlen");
    }
    match keys::lookup(view, scope, &args[1], Kind::Hash) {
        Err(error) => storage_error(error),
        Ok(Lookup::WrongType) => keys::wrong_type(),
        Ok(Lookup::Absent) => Resp::Integer(0),
        Ok(Lookup::Meta(meta)) => Resp::Integer(meta.count as i64),
    }
}
