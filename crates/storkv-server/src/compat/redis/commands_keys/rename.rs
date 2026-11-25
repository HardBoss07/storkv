//! Atomic Redis `RENAME` and `RENAMENX` commands.

use bytes::Bytes;

use super::super::RedisSession;
use super::super::codec::Resp;
use super::super::helpers::{storage_error, wrong_arity};
use super::super::keyspace::{self, Outcome};

impl RedisSession {
    pub(crate) fn rename(&self, args: &[Bytes]) -> Resp {
        self.rename_impl(args, false)
    }

    pub(crate) fn renamenx(&self, args: &[Bytes]) -> Resp {
        self.rename_impl(args, true)
    }

    fn rename_impl(&self, args: &[Bytes], only_if_absent: bool) -> Resp {
        let (Some(key), Some(new_key)) = (args.get(1), args.get(2)) else {
            return wrong_arity(if only_if_absent { "renamenx" } else { "rename" });
        };
        if args.len() != 3 {
            return wrong_arity(if only_if_absent { "renamenx" } else { "rename" });
        }
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        let db = self.db();
        match keyspace::atomic(db, self.server.is_primary(), |atomic| {
            let Some(value) = atomic.get(&scope, key)? else {
                return Ok(Outcome::Reply(Resp::error("ERR no such key")));
            };
            if key == new_key {
                return Ok(Outcome::Reply(if only_if_absent {
                    Resp::Integer(0)
                } else {
                    Resp::simple("OK")
                }));
            }
            if only_if_absent && atomic.get(&scope, new_key)?.is_some() {
                return Ok(Outcome::Reply(Resp::Integer(0)));
            }
            let expires = db
                .key_meta(&scope, key)?
                .map(|meta| meta.expires_at_ms)
                .unwrap_or(0);
            atomic.put(&scope, new_key, &value, expires);
            atomic.delete(&scope, key);
            Ok(Outcome::Commit(if only_if_absent {
                Resp::Integer(1)
            } else {
                Resp::simple("OK")
            }))
        }) {
            Ok(response) => response,
            Err(error) => storage_error(error),
        }
    }
}
