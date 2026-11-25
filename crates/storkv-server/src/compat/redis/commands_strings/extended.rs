//! Atomic extended Redis string commands.

use bytes::Bytes;

use super::super::RedisSession;
use super::super::codec::Resp;
use super::super::helpers::{GetExMode, parse_getex_options, storage_error, wrong_arity};
use super::super::keyspace::{self, Outcome};

impl RedisSession {
    pub(crate) fn getdel(&self, args: &[Bytes]) -> Resp {
        if args.len() != 2 {
            return wrong_arity("getdel");
        }
        let Some(key) = args.get(1) else {
            return wrong_arity("getdel");
        };
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), self.server.is_primary(), |atomic| {
            let Some(value) = atomic.get(&scope, key)? else {
                return Ok(Outcome::Reply(None));
            };
            atomic.delete(&scope, key);
            Ok(Outcome::Commit(Some(value)))
        }) {
            Ok(Some(value)) => Resp::bulk(value),
            Ok(None) => Resp::null(),
            Err(error) => storage_error(error),
        }
    }

    pub(crate) fn getex(&self, args: &[Bytes]) -> Resp {
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
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), self.server.is_primary(), |atomic| {
            let Some(value) = atomic.get(&scope, key)? else {
                return Ok(Outcome::Reply(None));
            };
            match mode {
                None => Ok(Outcome::Reply(Some(value))),
                Some(GetExMode::Persist) => {
                    atomic.put(&scope, key, &value, 0);
                    Ok(Outcome::Commit(Some(value)))
                }
                Some(GetExMode::ExpireAt(expires_at_ms)) => {
                    atomic.put(&scope, key, &value, expires_at_ms);
                    Ok(Outcome::Commit(Some(value)))
                }
            }
        }) {
            Ok(Some(value)) => Resp::bulk(value),
            Ok(None) => Resp::null(),
            Err(error) => storage_error(error),
        }
    }

    pub(crate) fn msetnx(&self, args: &[Bytes]) -> Resp {
        if args.len() < 3 || !(args.len() - 1).is_multiple_of(2) {
            return wrong_arity("msetnx");
        }
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), self.server.is_primary(), |atomic| {
            let mut index = 1;
            while index + 1 < args.len() {
                if atomic.get(&scope, &args[index])?.is_some() {
                    return Ok(Outcome::Reply(0));
                }
                index += 2;
            }
            let mut index = 1;
            while index + 1 < args.len() {
                atomic.put(&scope, &args[index], &args[index + 1], 0);
                index += 2;
            }
            Ok(Outcome::Commit(1))
        }) {
            Ok(flag) => Resp::Integer(flag),
            Err(error) => storage_error(error),
        }
    }
}
