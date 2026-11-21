//! Redis counter commands.

use bytes::Bytes;
use storkv_core::StorkvError;

use super::super::RedisSession;
use super::super::codec::Resp;
use super::super::helpers::{parse_i64, storage_error, wrong_arity};
use super::super::keyspace::{self, Outcome};

const MAX_RETRIES: usize = 16;

impl RedisSession {
    pub(crate) fn counter(&self, args: &[Bytes], delta: i64) -> Resp {
        self.counter_impl(args, delta)
    }

    pub(crate) fn counter_by(&self, args: &[Bytes], sign: i64) -> Resp {
        let Some(delta) = args.get(2).and_then(|arg| parse_i64(arg)) else {
            return Resp::error("ERR value is not an integer or out of range");
        };
        let Some(delta) = delta.checked_mul(sign) else {
            return Resp::error("ERR increment or decrement would overflow");
        };
        self.counter_impl(args, delta)
    }

    fn counter_impl(&self, args: &[Bytes], delta: i64) -> Resp {
        let Some(key) = args.get(1) else {
            return wrong_arity("incr");
        };
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        let db = self.db();
        let mut attempts = 0;
        loop {
            attempts += 1;
            let result = keyspace::atomic(db, self.server.is_primary(), |atomic| {
                let current = atomic.get(&scope, key)?;
                let base = parse_counter(current.as_deref())?;
                let next = base.checked_add(delta).ok_or_else(|| {
                    StorkvError::InvalidOperation("increment or decrement would overflow".into())
                })?;
                let expires = if current.is_some() {
                    db.key_meta(&scope, key)?
                        .map(|meta| meta.expires_at_ms)
                        .unwrap_or(0)
                } else {
                    0
                };
                atomic.put(&scope, key, next.to_string().as_bytes(), expires);
                Ok(Outcome::Commit(next))
            });
            match result {
                Ok(value) => return Resp::Integer(value),
                Err(StorkvError::WriteConflict { .. }) if attempts < MAX_RETRIES => continue,
                Err(error) => return storage_error(error),
            }
        }
    }
}

fn parse_counter(value: Option<&[u8]>) -> storkv_core::Result<i64> {
    match value {
        Some(value) => std::str::from_utf8(value)
            .ok()
            .and_then(|text| text.parse::<i64>().ok())
            .ok_or_else(|| {
                StorkvError::InvalidOperation("value is not an integer or out of range".into())
            }),
        None => Ok(0),
    }
}
