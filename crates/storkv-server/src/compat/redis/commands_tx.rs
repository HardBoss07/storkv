//! Redis `MULTI`/`EXEC`/`WATCH` implemented on one MVCC transaction.

use bytes::Bytes;
use storkv_core::{StorKvError, Transaction};

use super::codec::Resp;
use super::commands::{RedisSession, WatchState};
use super::helpers::{
    cursor_from_key, decode_cursor, glob_match, parse_i64, parse_set_options, parse_u64,
    storage_error, upper, wrong_arity,
};
use super::keyspace;

impl RedisSession {
    pub(super) fn watch(&mut self, args: &[Bytes]) -> Resp {
        if self.multi.is_some() {
            return Resp::error("ERR WATCH inside MULTI is not allowed");
        }
        if args.len() < 2 {
            return wrong_arity("watch");
        }
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        let read_seq = self
            .db()
            .inner()
            .last_visible_seq
            .load(std::sync::atomic::Ordering::Acquire);
        let state = self.watch.get_or_insert_with(|| WatchState {
            read_seq,
            keys: Vec::new(),
        });
        for key in &args[1..] {
            state.keys.push((scope.scope_id, key.clone()));
        }
        Resp::simple("OK")
    }

    pub(super) fn exec(&mut self) -> Resp {
        let Some(queue) = self.multi.take() else {
            return Resp::error("ERR EXEC without MULTI");
        };
        let had_error = std::mem::replace(&mut self.multi_error, false);
        let watch = self.watch.take();
        if had_error {
            return Resp::error("EXECABORT Transaction discarded because of previous errors.");
        }
        let db = self.server.db().clone();
        let mut transaction = match &watch {
            Some(state) => db.begin_tx_at(state.read_seq),
            None => db.begin_tx(),
        };
        if let Some(state) = &watch {
            for (scope_id, key) in &state.keys {
                transaction.record_read(*scope_id, key);
            }
        }
        let mut results = Vec::with_capacity(queue.len());
        for args in queue {
            let command = args.first().map(|bytes| upper(bytes)).unwrap_or_default();
            results.push(self.execute_in_tx(&mut transaction, &command, &args));
        }
        match transaction.commit() {
            Ok(()) => Resp::array(results),
            Err(StorKvError::WriteConflict { .. }) => Resp::Array(None),
            Err(error) => storage_error(error),
        }
    }

    fn execute_in_tx(
        &mut self,
        transaction: &mut Transaction,
        command: &str,
        args: &[Bytes],
    ) -> Resp {
        match command {
            "PING" | "ECHO" | "HELLO" | "SELECT" | "INFO" | "DBSIZE" => {
                self.execute_now(command, args)
            }
            "UNWATCH" => {
                self.watch = None;
                Resp::simple("OK")
            }
            "FLUSHDB" | "FLUSHALL" => Resp::error("ERR command is not allowed inside MULTI"),
            _ => self.execute_tx_data(transaction, command, args),
        }
    }

    fn execute_tx_data(&mut self, tx: &mut Transaction, command: &str, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match command {
            "GET" => tx_get(tx, &scope, args),
            "SET" => tx_set(self, tx, &scope, args),
            "SETEX" => tx_setex(tx, &scope, args, 1000),
            "PSETEX" => tx_setex(tx, &scope, args, 1),
            "SETNX" => tx_setnx(tx, &scope, args),
            "MGET" => tx_mget(tx, &scope, args),
            "MSET" => tx_mset(tx, &scope, args),
            "DEL" | "UNLINK" => tx_delete(tx, &scope, args),
            "EXISTS" => tx_exists(tx, &scope, args),
            "INCR" => tx_counter(self, tx, &scope, args, 1),
            "DECR" => tx_counter(self, tx, &scope, args, -1),
            "INCRBY" => tx_counter_by(self, tx, &scope, args, 1),
            "DECRBY" => tx_counter_by(self, tx, &scope, args, -1),
            "APPEND" => tx_append(self, tx, &scope, args),
            "STRLEN" => tx_strlen(tx, &scope, args),
            "EXPIRE" => tx_expire(tx, &scope, args, 1000),
            "PEXPIRE" => tx_expire(tx, &scope, args, 1),
            "PERSIST" => tx_persist(tx, &scope, args),
            "TTL" => tx_ttl(self, &scope, args, 1000),
            "PTTL" => tx_ttl(self, &scope, args, 1),
            "KEYS" => tx_keys(tx, &scope, args),
            "SCAN" => tx_scan(tx, &scope, args),
            other => Resp::error(format!("ERR unknown command '{other}'")),
        }
    }
}

fn tx_get(tx: &mut Transaction, scope: &storkv_core::ScopeHandle, args: &[Bytes]) -> Resp {
    let Some(key) = args.get(1) else {
        return wrong_arity("get");
    };
    match tx.get(scope, key) {
        Ok(Some(value)) => Resp::bulk(value),
        Ok(None) => Resp::null(),
        Err(error) => storage_error(error),
    }
}

fn tx_set(
    session: &RedisSession,
    tx: &mut Transaction,
    scope: &storkv_core::ScopeHandle,
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

fn tx_setex(
    tx: &mut Transaction,
    scope: &storkv_core::ScopeHandle,
    args: &[Bytes],
    scale: u64,
) -> Resp {
    let (Some(key), Some(ttl), Some(value)) = (args.get(1), args.get(2), args.get(3)) else {
        return wrong_arity("setex");
    };
    let Some(ttl) = parse_i64(ttl) else {
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

fn tx_setnx(tx: &mut Transaction, scope: &storkv_core::ScopeHandle, args: &[Bytes]) -> Resp {
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

fn tx_mget(tx: &mut Transaction, scope: &storkv_core::ScopeHandle, args: &[Bytes]) -> Resp {
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

fn tx_mset(tx: &mut Transaction, scope: &storkv_core::ScopeHandle, args: &[Bytes]) -> Resp {
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

fn tx_delete(tx: &mut Transaction, scope: &storkv_core::ScopeHandle, args: &[Bytes]) -> Resp {
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

fn tx_exists(tx: &mut Transaction, scope: &storkv_core::ScopeHandle, args: &[Bytes]) -> Resp {
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

fn tx_counter(
    session: &RedisSession,
    tx: &mut Transaction,
    scope: &storkv_core::ScopeHandle,
    args: &[Bytes],
    delta: i64,
) -> Resp {
    tx_counter_delta(session, tx, scope, args, delta)
}

fn tx_counter_by(
    session: &RedisSession,
    tx: &mut Transaction,
    scope: &storkv_core::ScopeHandle,
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
    scope: &storkv_core::ScopeHandle,
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

fn tx_append(
    session: &RedisSession,
    tx: &mut Transaction,
    scope: &storkv_core::ScopeHandle,
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

fn tx_strlen(tx: &mut Transaction, scope: &storkv_core::ScopeHandle, args: &[Bytes]) -> Resp {
    let Some(key) = args.get(1) else {
        return wrong_arity("strlen");
    };
    match tx.get(scope, key) {
        Ok(Some(value)) => Resp::Integer(value.len() as i64),
        Ok(None) => Resp::Integer(0),
        Err(error) => storage_error(error),
    }
}

fn tx_expire(
    tx: &mut Transaction,
    scope: &storkv_core::ScopeHandle,
    args: &[Bytes],
    scale: u64,
) -> Resp {
    let (Some(key), Some(ttl)) = (args.get(1), args.get(2)) else {
        return wrong_arity("expire");
    };
    let Some(ttl) = parse_i64(ttl) else {
        return Resp::error("ERR value is not an integer or out of range");
    };
    let value = match tx.get(scope, key) {
        Ok(Some(value)) => value,
        Ok(None) => return Resp::Integer(0),
        Err(error) => return storage_error(error),
    };
    if ttl <= 0 {
        return match tx.delete(scope, key) {
            Ok(()) => Resp::Integer(1),
            Err(error) => storage_error(error),
        };
    }
    let ttl = std::time::Duration::from_millis((ttl as u64).saturating_mul(scale));
    match tx.put_with_expires_at(scope, key, &value, keyspace::expiry_from_ttl(Some(ttl))) {
        Ok(()) => Resp::Integer(1),
        Err(error) => storage_error(error),
    }
}

fn tx_persist(tx: &mut Transaction, scope: &storkv_core::ScopeHandle, args: &[Bytes]) -> Resp {
    let Some(key) = args.get(1) else {
        return wrong_arity("persist");
    };
    let value = match tx.get(scope, key) {
        Ok(Some(value)) => value,
        Ok(None) => return Resp::Integer(0),
        Err(error) => return storage_error(error),
    };
    match tx.put_with_expires_at(scope, key, &value, 0) {
        Ok(()) => Resp::Integer(1),
        Err(error) => storage_error(error),
    }
}

fn tx_ttl(
    session: &RedisSession,
    scope: &storkv_core::ScopeHandle,
    args: &[Bytes],
    scale: u64,
) -> Resp {
    let Some(key) = args.get(1) else {
        return wrong_arity("ttl");
    };
    match keyspace::remaining_ms(session.db(), scope, key) {
        Ok(None) => Resp::Integer(-2),
        Ok(Some(0)) => Resp::Integer(-1),
        Ok(Some(remaining)) if scale == 1000 => Resp::Integer(remaining.div_ceil(1000) as i64),
        Ok(Some(remaining)) => Resp::Integer(remaining as i64),
        Err(error) => storage_error(error),
    }
}

fn tx_keys(tx: &mut Transaction, scope: &storkv_core::ScopeHandle, args: &[Bytes]) -> Resp {
    let Some(pattern) = args.get(1) else {
        return wrong_arity("keys");
    };
    match tx.scan(scope, None::<&[u8]>, None::<&[u8]>) {
        Ok(pairs) => Resp::array(
            pairs
                .into_iter()
                .filter(|(key, _)| glob_match(pattern, key))
                .map(|(key, _)| Resp::bulk(key))
                .collect(),
        ),
        Err(error) => storage_error(error),
    }
}

fn tx_scan(tx: &mut Transaction, scope: &storkv_core::ScopeHandle, args: &[Bytes]) -> Resp {
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
    for (key, _) in pairs {
        let matches = pattern.is_none_or(|pattern| glob_match(pattern, &key));
        if matches && keys.len() < count {
            keys.push(Resp::bulk(key));
        } else if matches {
            next_cursor = Some(cursor_from_key(&key));
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
