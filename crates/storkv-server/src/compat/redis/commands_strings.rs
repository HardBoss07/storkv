//! Redis string commands.

use bytes::Bytes;

use super::RedisSession;
use super::codec::Resp;
use super::helpers::{parse_i64, parse_set_options, storage_error, wrong_arity};
use super::keyspace::{self, Outcome};

mod counter;

impl RedisSession {
    pub(super) fn get(&self, args: &[Bytes]) -> Resp {
        let Some(key) = args.get(1) else {
            return wrong_arity("get");
        };
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match self.db().get(&scope, key) {
            Ok(Some(value)) => Resp::bulk(value),
            Ok(None) => Resp::null(),
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn set(&self, args: &[Bytes]) -> Resp {
        let (Some(key), Some(value)) = (args.get(1), args.get(2)) else {
            return wrong_arity("set");
        };
        let options = match parse_set_options(args) {
            Ok(options) => options,
            Err(response) => return response,
        };
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        let db = self.db();
        let result = keyspace::atomic(db, |atomic| {
            let previous = atomic.get(&scope, key)?;
            if options.only_if_absent && previous.is_some() {
                return Ok(Outcome::Reply((previous, false)));
            }
            if options.only_if_present && previous.is_none() {
                return Ok(Outcome::Reply((None, false)));
            }
            let expires = if options.keep_ttl {
                db.key_meta(&scope, key)?
                    .map(|meta| meta.expires_at_ms)
                    .unwrap_or(0)
            } else {
                keyspace::expiry_from_ttl(options.ttl)
            };
            atomic.put(&scope, key, value, expires);
            Ok(Outcome::Commit((previous, true)))
        });
        match result {
            Ok((previous, written)) => set_reply(options.return_previous, previous, written),
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn setex(&self, args: &[Bytes], seconds_scale: u64) -> Resp {
        let (Some(key), Some(ttl), Some(value)) = (args.get(1), args.get(2), args.get(3)) else {
            return wrong_arity("setex");
        };
        let Some(ttl) = parse_i64(ttl) else {
            return Resp::error("ERR invalid expire time");
        };
        if ttl <= 0 {
            return Resp::error("ERR invalid expire time");
        }
        let ttl = std::time::Duration::from_millis((ttl as u64).saturating_mul(seconds_scale));
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), |atomic| {
            atomic.put(&scope, key, value, keyspace::expiry_from_ttl(Some(ttl)));
            Ok(Outcome::Commit(()))
        }) {
            Ok(()) => Resp::simple("OK"),
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn setnx(&self, args: &[Bytes]) -> Resp {
        let (Some(key), Some(value)) = (args.get(1), args.get(2)) else {
            return wrong_arity("setnx");
        };
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), |atomic| {
            if atomic.get(&scope, key)?.is_some() {
                return Ok(Outcome::Reply(0));
            }
            atomic.put(&scope, key, value, 0);
            Ok(Outcome::Commit(1))
        }) {
            Ok(flag) => Resp::Integer(flag),
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn mget(&self, args: &[Bytes]) -> Resp {
        if args.len() < 2 {
            return wrong_arity("mget");
        }
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        let mut values = Vec::with_capacity(args.len() - 1);
        for key in &args[1..] {
            match self.db().get(&scope, key) {
                Ok(Some(value)) => values.push(Resp::bulk(value)),
                Ok(None) => values.push(Resp::null()),
                Err(error) => return storage_error(error),
            }
        }
        Resp::array(values)
    }

    pub(super) fn mset(&self, args: &[Bytes]) -> Resp {
        if args.len() < 3 || (args.len() - 1) % 2 != 0 {
            return wrong_arity("mset");
        }
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), |atomic| {
            let mut index = 1;
            while index + 1 < args.len() {
                atomic.put(&scope, &args[index], &args[index + 1], 0);
                index += 2;
            }
            Ok(Outcome::Commit(()))
        }) {
            Ok(()) => Resp::simple("OK"),
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn append(&self, args: &[Bytes]) -> Resp {
        let (Some(key), Some(suffix)) = (args.get(1), args.get(2)) else {
            return wrong_arity("append");
        };
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        let db = self.db();
        match keyspace::atomic(db, |atomic| {
            let current = atomic.get(&scope, key)?.unwrap_or_default();
            let mut combined = Vec::with_capacity(current.len() + suffix.len());
            combined.extend_from_slice(&current);
            combined.extend_from_slice(suffix);
            let expires = if current.is_empty() {
                0
            } else {
                db.key_meta(&scope, key)?
                    .map(|meta| meta.expires_at_ms)
                    .unwrap_or(0)
            };
            atomic.put(&scope, key, &combined, expires);
            Ok(Outcome::Commit(combined.len()))
        }) {
            Ok(length) => Resp::Integer(length as i64),
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn strlen(&self, args: &[Bytes]) -> Resp {
        let Some(key) = args.get(1) else {
            return wrong_arity("strlen");
        };
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match self.db().get(&scope, key) {
            Ok(Some(value)) => Resp::Integer(value.len() as i64),
            Ok(None) => Resp::Integer(0),
            Err(error) => storage_error(error),
        }
    }
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
