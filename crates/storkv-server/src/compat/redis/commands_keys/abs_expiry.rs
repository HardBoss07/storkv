//! Redis absolute expiry commands with `NX`, `XX`, `GT`, and `LT` flags.

use bytes::Bytes;
use storkv_core::TimestampMs;

use super::super::RedisSession;
use super::super::codec::Resp;
use super::super::helpers::{parse_expire_conditions, parse_i64, storage_error, wrong_arity};
use super::super::keyspace::{self, Outcome};

impl RedisSession {
    pub(crate) fn expire_at(&self, args: &[Bytes], scale: u64) -> Resp {
        let command = if scale == super::super::commands::MILLIS_PER_SECOND {
            "expireat"
        } else {
            "pexpireat"
        };
        let (Some(key), Some(value)) = (args.get(1), args.get(2)) else {
            return wrong_arity(command);
        };
        let Some(value) = parse_i64(value) else {
            return Resp::error("ERR value is not an integer or out of range");
        };
        let conditions = match parse_expire_conditions(args, 3) {
            Ok(conditions) => conditions,
            Err(response) => return response,
        };
        let target = (value.max(0) as u64).saturating_mul(scale);
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), self.server.is_primary(), |atomic| {
            let Some(stored) = atomic.get(&scope, key)? else {
                return Ok(Outcome::Reply(0));
            };
            let current = atomic.expires_at(&scope, key)?.unwrap_or(0);
            if !conditions.matches(current, target) {
                return Ok(Outcome::Reply(0));
            }
            if target <= TimestampMs::now().get() {
                atomic.delete(&scope, key);
            } else {
                atomic.put(&scope, key, &stored, target);
            }
            Ok(Outcome::Commit(1))
        }) {
            Ok(flag) => Resp::Integer(flag),
            Err(error) => storage_error(error),
        }
    }
}
