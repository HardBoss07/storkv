//! Redis hash and set commands run on the current scope.

use bytes::Bytes;

use super::RedisSession;
use super::codec::Resp;
use super::composite::{self, DbView};
use super::helpers::storage_error;
use super::keyspace;

impl RedisSession {
    pub(super) fn hset(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), self.server.is_primary(), |view| {
            composite::hset(view, &scope, args)
        }) {
            Ok(reply) => reply,
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn hdel(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), self.server.is_primary(), |view| {
            composite::hdel(view, &scope, args)
        }) {
            Ok(reply) => reply,
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn hincrby(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), self.server.is_primary(), |view| {
            composite::hincrby(view, &scope, args)
        }) {
            Ok(reply) => reply,
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn sadd(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), self.server.is_primary(), |view| {
            composite::sadd(view, &scope, args)
        }) {
            Ok(reply) => reply,
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn srem(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), self.server.is_primary(), |view| {
            composite::srem(view, &scope, args)
        }) {
            Ok(reply) => reply,
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn hget(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        composite::hget(&mut DbView(self.db()), &scope, args)
    }

    pub(super) fn hmget(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        composite::hmget(&mut DbView(self.db()), &scope, args)
    }

    pub(super) fn hgetall(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        composite::hgetall(&mut DbView(self.db()), &scope, args)
    }

    pub(super) fn hexists(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        composite::hexists(&mut DbView(self.db()), &scope, args)
    }

    pub(super) fn hlen(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        composite::hlen(&mut DbView(self.db()), &scope, args)
    }

    pub(super) fn hkeys(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        composite::hkeys(&mut DbView(self.db()), &scope, args)
    }

    pub(super) fn hvals(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        composite::hvals(&mut DbView(self.db()), &scope, args)
    }

    pub(super) fn smembers(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        composite::smembers(&mut DbView(self.db()), &scope, args)
    }

    pub(super) fn sismember(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        composite::sismember(&mut DbView(self.db()), &scope, args)
    }

    pub(super) fn scard(&self, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        composite::scard(&mut DbView(self.db()), &scope, args)
    }

    /// Dispatch a hash or set command, or return `None` for other commands.
    pub(super) fn collection_command(&self, command: &str, args: &[Bytes]) -> Option<Resp> {
        Some(match command {
            "HSET" => self.hset(args),
            "HGET" => self.hget(args),
            "HMGET" => self.hmget(args),
            "HGETALL" => self.hgetall(args),
            "HDEL" => self.hdel(args),
            "HINCRBY" => self.hincrby(args),
            "HEXISTS" => self.hexists(args),
            "HLEN" => self.hlen(args),
            "HKEYS" => self.hkeys(args),
            "HVALS" => self.hvals(args),
            "SADD" => self.sadd(args),
            "SREM" => self.srem(args),
            "SMEMBERS" => self.smembers(args),
            "SISMEMBER" => self.sismember(args),
            "SCARD" => self.scard(args),
            _ => return None,
        })
    }
}
