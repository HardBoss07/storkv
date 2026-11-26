//! Key scan commands inside a Redis transaction.

use bytes::Bytes;
use storkv_core::{ScopeHandle, Transaction};

use super::super::codec::Resp;
use super::super::composite;
use super::super::helpers::{
    cursor_from_key, decode_cursor, glob_match, parse_u64, storage_error, upper,
    validate_glob_pattern, wrong_arity,
};

pub(super) fn tx_keys(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    let Some(pattern) = args.get(1) else {
        return wrong_arity("keys");
    };
    if let Err(response) = validate_glob_pattern(pattern) {
        return response;
    }
    match tx.scan(scope, None::<&[u8]>, None::<&[u8]>) {
        Ok(pairs) => Resp::array(
            pairs
                .into_iter()
                .filter_map(|(key, _)| composite::logical_key(&key))
                .filter(|key| glob_match(pattern, key))
                .map(Resp::bulk)
                .collect(),
        ),
        Err(error) => storage_error(error),
    }
}

pub(super) fn tx_scan(tx: &mut Transaction, scope: &ScopeHandle, args: &[Bytes]) -> Resp {
    let Some(cursor) = args.get(1) else {
        return wrong_arity("scan");
    };
    let start = match decode_cursor(cursor) {
        Some(start) => start,
        None => return Resp::error("ERR invalid cursor"),
    };
    let (pattern, count) = match parse_scan_options(args) {
        Ok(options) => options,
        Err(response) => return response,
    };
    let pairs = match tx.scan(scope, start.as_deref(), None::<&[u8]>) {
        Ok(pairs) => pairs,
        Err(error) => return storage_error(error),
    };
    let mut keys = Vec::new();
    let mut next_cursor = None;
    for (raw, _) in pairs {
        let Some(key) = composite::logical_key(&raw) else {
            continue;
        };
        let matches = pattern.is_none_or(|pattern| glob_match(pattern, &key));
        if matches && keys.len() < count {
            keys.push(Resp::bulk(key));
        } else if matches {
            next_cursor = Some(cursor_from_key(&raw));
            break;
        }
    }
    let cursor = next_cursor.unwrap_or_else(|| Bytes::from_static(b"0"));
    Resp::array(vec![Resp::bulk(cursor), Resp::array(keys)])
}

fn parse_scan_options(args: &[Bytes]) -> Result<(Option<&[u8]>, usize), Resp> {
    let mut pattern: Option<&[u8]> = None;
    let mut count = 10usize;
    let mut index = 2;
    while index < args.len() {
        match upper(&args[index]).as_str() {
            "MATCH" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(Resp::error("ERR syntax error"));
                };
                validate_glob_pattern(value)?;
                pattern = Some(value);
                index += 2;
            }
            "COUNT" => {
                let Some(value) = args.get(index + 1).and_then(|arg| parse_u64(arg)) else {
                    return Err(Resp::error("ERR syntax error"));
                };
                count = usize::try_from(value).unwrap_or(usize::MAX).max(1);
                index += 2;
            }
            _ => return Err(Resp::error("ERR syntax error")),
        }
    }
    Ok((pattern, count))
}
