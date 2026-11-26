//! Redis key-space commands.

use bytes::Bytes;

use super::RedisSession;
use super::codec::Resp;
use super::commands::MILLIS_PER_SECOND;
use super::composite;
use super::helpers::{
    cursor_from_key, decode_cursor, glob_match, parse_i64, parse_u64, storage_error, upper,
    validate_glob_pattern, wrong_arity,
};
use super::keyspace::{self, Outcome};

mod abs_expiry;
mod key_info;
mod rename;

impl RedisSession {
    pub(super) fn delete(&self, args: &[Bytes]) -> Resp {
        if args.len() < 2 {
            return wrong_arity("del");
        }
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), self.server.is_primary(), |atomic| {
            let mut removed = 0;
            for key in &args[1..] {
                if atomic.get(&scope, key)?.is_some() {
                    atomic.delete(&scope, key);
                    removed += 1;
                }
            }
            Ok(Outcome::Commit(removed))
        }) {
            Ok(removed) => Resp::Integer(removed),
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn exists(&self, args: &[Bytes]) -> Resp {
        if args.len() < 2 {
            return wrong_arity("exists");
        }
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        let mut count = 0;
        for key in &args[1..] {
            match self.db().get(&scope, key) {
                Ok(Some(_)) => count += 1,
                Ok(None) => {}
                Err(error) => return storage_error(error),
            }
        }
        Resp::Integer(count)
    }

    pub(super) fn expire(&self, args: &[Bytes], scale: u64) -> Resp {
        let (Some(key), Some(ttl)) = (args.get(1), args.get(2)) else {
            return wrong_arity("expire");
        };
        let Some(ttl) = parse_i64(ttl) else {
            return Resp::error("ERR value is not an integer or out of range");
        };
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), self.server.is_primary(), |atomic| {
            let Some(value) = atomic.get(&scope, key)? else {
                return Ok(Outcome::Reply(0));
            };
            if ttl <= 0 {
                atomic.delete(&scope, key);
                return Ok(Outcome::Commit(1));
            }
            let ttl = std::time::Duration::from_millis((ttl as u64).saturating_mul(scale));
            atomic.put(&scope, key, &value, keyspace::expiry_from_ttl(Some(ttl)));
            Ok(Outcome::Commit(1))
        }) {
            Ok(flag) => Resp::Integer(flag),
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn persist(&self, args: &[Bytes]) -> Resp {
        let Some(key) = args.get(1) else {
            return wrong_arity("persist");
        };
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), self.server.is_primary(), |atomic| {
            let Some(value) = atomic.get(&scope, key)? else {
                return Ok(Outcome::Reply(0));
            };
            atomic.put(&scope, key, &value, 0);
            Ok(Outcome::Commit(1))
        }) {
            Ok(flag) => Resp::Integer(flag),
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn ttl(&self, args: &[Bytes], scale: u64) -> Resp {
        let Some(key) = args.get(1) else {
            return wrong_arity("ttl");
        };
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::remaining_ms(self.db(), &scope, key) {
            Ok(None) => Resp::Integer(-2),
            Ok(Some(0)) => Resp::Integer(-1),
            Ok(Some(remaining)) => Resp::Integer(ttl_value(remaining, scale)),
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn keys(&self, args: &[Bytes]) -> Resp {
        let Some(pattern) = args.get(1) else {
            return wrong_arity("keys");
        };
        if let Err(response) = validate_glob_pattern(pattern) {
            return response;
        }
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match self.db().scan_with(
            &scope,
            None::<&[u8]>,
            None::<&[u8]>,
            storkv_core::ScanOptions {
                keys_only: true,
                ..Default::default()
            },
        ) {
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

    pub(super) fn scan(&self, args: &[Bytes]) -> Resp {
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
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        // shortcut: COUNT is a hint, so the core limit is a small multiple of it.
        // Upgrade when MATCH filters can stop inside the engine.
        let pairs = match self.db().scan_with(
            &scope,
            start.as_deref(),
            None::<&[u8]>,
            storkv_core::ScanOptions {
                limit: Some(count.saturating_mul(8).max(count)),
                keys_only: true,
                ..Default::default()
            },
        ) {
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
}

fn ttl_value(remaining: u64, scale: u64) -> i64 {
    if scale == MILLIS_PER_SECOND {
        remaining.div_ceil(MILLIS_PER_SECOND) as i64
    } else {
        remaining as i64
    }
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
