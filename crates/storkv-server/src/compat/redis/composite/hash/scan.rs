//! Hash enumeration commands shared by direct and transactional commands.

use bytes::Bytes;
use storkv_core::ScopeHandle;

use super::super::super::codec::Resp;
use super::super::super::helpers::{storage_error, wrong_arity};
use super::super::keys::{self, Kind, Lookup};
use super::super::view::ReadView;

/// Run `HGETALL`.
pub(crate) fn hgetall<V: ReadView>(view: &mut V, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() != 2 {
        return wrong_arity("hgetall");
    }
    match fields(view, scope, &args[1]) {
        Ok(pairs) => {
            let mut flat = Vec::with_capacity(pairs.len() * 2);
            for (field, value) in pairs {
                flat.push(Resp::bulk(field));
                flat.push(Resp::bulk(value));
            }
            Resp::array(flat)
        }
        Err(response) => response,
    }
}

/// Run `HKEYS`.
pub(crate) fn hkeys<V: ReadView>(view: &mut V, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() != 2 {
        return wrong_arity("hkeys");
    }
    match fields(view, scope, &args[1]) {
        Ok(pairs) => Resp::array(
            pairs
                .into_iter()
                .map(|(field, _)| Resp::bulk(field))
                .collect(),
        ),
        Err(response) => response,
    }
}

/// Run `HVALS`.
pub(crate) fn hvals<V: ReadView>(view: &mut V, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    if args.len() != 2 {
        return wrong_arity("hvals");
    }
    match fields(view, scope, &args[1]) {
        Ok(pairs) => Resp::array(
            pairs
                .into_iter()
                .map(|(_, value)| Resp::bulk(value))
                .collect(),
        ),
        Err(response) => response,
    }
}

fn fields<V: ReadView>(
    view: &mut V,
    scope: &ScopeHandle,
    key: &[u8],
) -> Result<Vec<(Bytes, Bytes)>, Resp> {
    let prefix = keys::data_prefix(Kind::Hash, key);
    match keys::lookup(view, scope, key, Kind::Hash) {
        Err(error) => Err(storage_error(error)),
        Ok(Lookup::WrongType) => Err(keys::wrong_type()),
        Ok(Lookup::Absent) => Ok(Vec::new()),
        Ok(Lookup::Meta(_)) => match view.scan_prefix(scope, &prefix) {
            Ok(pairs) => Ok(pairs
                .into_iter()
                .map(|(raw, value)| (raw.slice(prefix.len()..), value))
                .collect()),
            Err(error) => Err(storage_error(error)),
        },
    }
}
