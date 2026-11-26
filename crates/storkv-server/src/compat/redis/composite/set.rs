//! Generic set command logic shared by direct and transactional commands.

use bytes::Bytes;
use storkv_core::{ScopeHandle, StorkvError};

use super::super::codec::Resp;
use super::super::helpers::{storage_error, wrong_arity};
use super::super::keyspace::Outcome;
use super::keys::{self, Kind, Lookup};
use super::view::{ReadView, WriteView};

/// Run `SADD`.
pub(crate) fn sadd<V: WriteView>(
    view: &mut V,
    scope: &ScopeHandle,
    args: &[Bytes],
) -> Result<Outcome<Resp>, StorkvError> {
    if args.len() < 3 {
        return Ok(Outcome::Reply(wrong_arity("sadd")));
    }
    let key = &args[1];
    let (mut count, existed) = match keys::lookup(view, scope, key, Kind::Set)? {
        Lookup::Absent => (0, false),
        Lookup::Meta(meta) => (meta.count, true),
        Lookup::WrongType => return Ok(Outcome::Reply(keys::wrong_type())),
    };
    let mut added = 0u64;
    for member in &args[2..] {
        let member_key = keys::data_key(Kind::Set, key, member);
        if view.get(scope, &member_key)?.is_none() {
            added += 1;
        }
        view.put(scope, &member_key, b"", 0)?;
    }
    if added > 0 || !existed {
        count = count.saturating_add(added);
        keys::write_meta(view, scope, key, Kind::Set, count)?;
    }
    Ok(Outcome::Commit(Resp::Integer(added as i64)))
}

/// Run `SREM`.
pub(crate) fn srem<V: WriteView>(
    view: &mut V,
    scope: &ScopeHandle,
    args: &[Bytes],
) -> Result<Outcome<Resp>, StorkvError> {
    if args.len() < 3 {
        return Ok(Outcome::Reply(wrong_arity("srem")));
    }
    let key = &args[1];
    let meta = match keys::lookup(view, scope, key, Kind::Set)? {
        Lookup::Absent => return Ok(Outcome::Commit(Resp::Integer(0))),
        Lookup::Meta(meta) => meta,
        Lookup::WrongType => return Ok(Outcome::Reply(keys::wrong_type())),
    };
    let mut removed = 0u64;
    for member in &args[2..] {
        let member_key = keys::data_key(Kind::Set, key, member);
        if view.get(scope, &member_key)?.is_some() {
            view.delete(scope, &member_key)?;
            removed += 1;
        }
    }
    if removed > 0 {
        let remaining = meta.count.saturating_sub(removed);
        if remaining == 0 {
            keys::delete_meta(view, scope, key)?;
        } else {
            keys::write_meta(view, scope, key, Kind::Set, remaining)?;
        }
    }
    Ok(Outcome::Commit(Resp::Integer(removed as i64)))
}

/// Run `SMEMBERS`.
pub(crate) fn smembers<V: ReadView>(view: &mut V, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() != 2 {
        return wrong_arity("smembers");
    }
    let key = &args[1];
    let prefix = keys::data_prefix(Kind::Set, key);
    match keys::lookup(view, scope, key, Kind::Set) {
        Err(error) => storage_error(error),
        Ok(Lookup::WrongType) => keys::wrong_type(),
        Ok(Lookup::Absent) => Resp::array(Vec::new()),
        Ok(Lookup::Meta(_)) => match view.scan_prefix(scope, &prefix) {
            Ok(pairs) => Resp::array(
                pairs
                    .into_iter()
                    .map(|(raw, _)| Resp::bulk(raw.slice(prefix.len()..)))
                    .collect(),
            ),
            Err(error) => storage_error(error),
        },
    }
}

/// Run `SISMEMBER`.
pub(crate) fn sismember<V: ReadView>(view: &mut V, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() != 3 {
        return wrong_arity("sismember");
    }
    match keys::lookup(view, scope, &args[1], Kind::Set) {
        Err(error) => storage_error(error),
        Ok(Lookup::WrongType) => keys::wrong_type(),
        Ok(Lookup::Absent) => Resp::Integer(0),
        Ok(Lookup::Meta(_)) => {
            let member_key = keys::data_key(Kind::Set, &args[1], &args[2]);
            match view.get(scope, &member_key) {
                Ok(Some(_)) => Resp::Integer(1),
                Ok(None) => Resp::Integer(0),
                Err(error) => storage_error(error),
            }
        }
    }
}

/// Run `SCARD`.
pub(crate) fn scard<V: ReadView>(view: &mut V, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() != 2 {
        return wrong_arity("scard");
    }
    match keys::lookup(view, scope, &args[1], Kind::Set) {
        Err(error) => storage_error(error),
        Ok(Lookup::WrongType) => keys::wrong_type(),
        Ok(Lookup::Absent) => Resp::Integer(0),
        Ok(Lookup::Meta(meta)) => Resp::Integer(meta.count as i64),
    }
}
