//! Redis `TYPE` and absolute expiration reporting commands.

use bytes::Bytes;

use super::super::RedisSession;
use super::super::codec::Resp;
use super::super::composite::{self, DbView};
use super::super::helpers::{storage_error, wrong_arity};

impl RedisSession {
    pub(crate) fn key_type(&self, args: &[Bytes]) -> Resp {
        if args.len() != 2 {
            return wrong_arity("type");
        }
        let Some(key) = args.get(1) else {
            return wrong_arity("type");
        };
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match composite::read_kind(&mut DbView(self.db()), &scope, key) {
            Ok(Some(kind)) => return Resp::simple(kind.name()),
            Ok(None) => {}
            Err(error) => return storage_error(error),
        }
        match self.db().get(&scope, key) {
            Ok(Some(_)) => Resp::simple("string"),
            Ok(None) => Resp::simple("none"),
            Err(error) => storage_error(error),
        }
    }

    pub(crate) fn expiretime(&self, args: &[Bytes], scale: u64) -> Resp {
        let command = if scale == super::super::commands::MILLIS_PER_SECOND {
            "expiretime"
        } else {
            "pexpiretime"
        };
        if args.len() != 2 {
            return wrong_arity(command);
        }
        let Some(key) = args.get(1) else {
            return wrong_arity(command);
        };
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match self.db().key_meta(&scope, key) {
            Ok(Some(meta)) if meta.expires_at_ms == 0 => Resp::Integer(-1),
            Ok(Some(meta)) => Resp::Integer((meta.expires_at_ms / scale) as i64),
            Ok(None) => Resp::Integer(-2),
            Err(error) => storage_error(error),
        }
    }
}
