//! Redis command dispatch and per-connection session state.

use std::sync::Arc;

use bytes::Bytes;
use storkv_core::{Db, ScopeHandle};

use super::codec::{RESP2_VERSION, Resp};
use super::helpers::{is_known_command, readonly_error, storage_error, upper};
use crate::compat;
use crate::server::Server;

/// Milliseconds in one second, used to scale Redis expiry values.
pub(super) const MILLIS_PER_SECOND: u64 = 1_000;

pub(super) struct WatchState {
    pub(super) read_seq: u64,
    pub(super) keys: Vec<(u32, Bytes)>,
}

pub(crate) struct RedisSession {
    pub(super) server: Arc<Server>,
    pub(super) multi: Option<Vec<Vec<Bytes>>>,
    pub(super) watch: Option<WatchState>,
    pub(super) database: u32,
    pub(super) resp_version: u8,
    pub(super) multi_error: bool,
    pub(super) authenticated: bool,
    pub(super) client_id: u64,
    pub(super) client_name: Option<String>,
    pub(crate) close: bool,
}

const _: () = assert!(std::mem::size_of::<RedisSession>() <= 128);

impl RedisSession {
    pub(crate) fn new(server: Arc<Server>) -> Self {
        let authenticated = server.redis_password().is_none();
        let client_id = server.next_redis_client_id();
        Self {
            server,
            database: 0,
            resp_version: RESP2_VERSION,
            multi: None,
            multi_error: false,
            authenticated,
            client_id,
            client_name: None,
            watch: None,
            close: false,
        }
    }

    pub(crate) fn resp_version(&self) -> u8 {
        self.resp_version
    }

    pub(crate) fn execute(&mut self, args: Vec<Bytes>) -> Resp {
        let Some(command_bytes) = args.first() else {
            return Resp::error("ERR empty command");
        };
        let command = upper(command_bytes);
        if !self.authenticated && !allows_before_auth(&command) {
            return Resp::error("NOAUTH Authentication required.");
        }
        if self.multi.is_some() {
            return self.queue_command(&command, args);
        }
        self.execute_now(&command, &args)
    }

    fn queue_command(&mut self, command: &str, args: Vec<Bytes>) -> Resp {
        match command {
            "MULTI" => return Resp::error("ERR MULTI calls can not be nested"),
            "EXEC" => return self.exec(),
            "DISCARD" => {
                self.multi = None;
                self.multi_error = false;
                self.watch = None;
                return Resp::simple("OK");
            }
            "WATCH" => return Resp::error("ERR WATCH inside MULTI is not allowed"),
            _ => {}
        }
        if !is_known_command(command) {
            self.multi_error = true;
            let name = args.first().cloned().unwrap_or_default();
            return Resp::error(format!(
                "ERR unknown command '{}'",
                String::from_utf8_lossy(&name)
            ));
        }
        if let Some(queue) = &mut self.multi {
            queue.push(args);
        }
        Resp::simple("QUEUED")
    }

    pub(super) fn execute_now(&mut self, command: &str, args: &[Bytes]) -> Resp {
        if let Err(response) = self.writable(command) {
            return response;
        }
        let result = match command {
            "AUTH" => self.auth(args),
            "CLIENT" => self.client_command(args),
            "COMMAND" => self.command_meta(args),
            "PING" => self.ping(args),
            "ECHO" => self.echo(args),
            "QUIT" => {
                self.close = true;
                Resp::simple("OK")
            }
            "HELLO" => self.hello(args),
            "SELECT" => self.select(args),
            "INFO" => self.info(),
            "DBSIZE" => self.dbsize(),
            "FLUSHDB" => self.flush_db(),
            "FLUSHALL" => self.flush_all(),
            "GET" => self.get(args),
            "GETDEL" => self.getdel(args),
            "GETEX" => self.getex(args),
            "SET" => self.set(args),
            "SETEX" => self.setex(args, MILLIS_PER_SECOND),
            "PSETEX" => self.setex(args, 1),
            "SETNX" => self.setnx(args),
            "MGET" => self.mget(args),
            "MSET" => self.mset(args),
            "MSETNX" => self.msetnx(args),
            "DEL" | "UNLINK" => self.delete(args),
            "EXISTS" => self.exists(args),
            "RENAME" => self.rename(args),
            "RENAMENX" => self.renamenx(args),
            "TYPE" => self.key_type(args),
            "INCR" => self.counter(args, 1),
            "DECR" => self.counter(args, -1),
            "INCRBY" => self.counter_by(args, 1),
            "DECRBY" => self.counter_by(args, -1),
            "APPEND" => self.append(args),
            "STRLEN" => self.strlen(args),
            "HSET" | "HGET" | "HMGET" | "HGETALL" | "HDEL" | "HINCRBY" | "HEXISTS" | "HLEN"
            | "HKEYS" | "HVALS" | "SADD" | "SREM" | "SMEMBERS" | "SISMEMBER" | "SCARD" => self
                .collection_command(command, args)
                .unwrap_or_else(|| Resp::error(format!("ERR unknown command '{command}'"))),
            "EXPIRE" => self.expire(args, MILLIS_PER_SECOND),
            "PEXPIRE" => self.expire(args, 1),
            "EXPIREAT" => self.expire_at(args, MILLIS_PER_SECOND),
            "PEXPIREAT" => self.expire_at(args, 1),
            "EXPIRETIME" => self.expiretime(args, MILLIS_PER_SECOND),
            "PEXPIRETIME" => self.expiretime(args, 1),
            "PERSIST" => self.persist(args),
            "TTL" => self.ttl(args, MILLIS_PER_SECOND),
            "PTTL" => self.ttl(args, 1),
            "KEYS" => self.keys(args),
            "SCAN" => self.scan(args),
            "MULTI" => {
                self.multi = Some(Vec::new());
                self.multi_error = false;
                Resp::simple("OK")
            }
            "EXEC" => Resp::error("ERR EXEC without MULTI"),
            "DISCARD" => Resp::error("ERR DISCARD without MULTI"),
            "WATCH" => self.watch(args),
            "UNWATCH" => {
                self.watch = None;
                Resp::simple("OK")
            }
            other => Resp::error(format!("ERR unknown command '{other}'")),
        };
        if records_transport_request(command) {
            self.server
                .stats
                .record_transport_request(crate::transports::TransportKind::Redis.as_u8());
        }
        result
    }

    pub(super) fn db(&self) -> &Db {
        self.server.db()
    }

    pub(super) fn scope(&self) -> Result<ScopeHandle, Resp> {
        compat::redis_scope(self.db(), self.server.is_primary(), self.database)
            .map_err(storage_error)
    }

    fn writable(&self, command: &str) -> Result<(), Resp> {
        if !is_write_command(command) {
            return Ok(());
        }
        self.server.ensure_primary().map_err(|_| readonly_error())
    }
}

/// Return `true` for commands that a connection may run before `AUTH`.
fn allows_before_auth(command: &str) -> bool {
    matches!(command, "AUTH" | "HELLO" | "PING")
}

fn is_write_command(command: &str) -> bool {
    matches!(
        command,
        "SET"
            | "SETEX"
            | "PSETEX"
            | "SETNX"
            | "MSET"
            | "MSETNX"
            | "GETDEL"
            | "GETEX"
            | "RENAME"
            | "RENAMENX"
            | "DEL"
            | "UNLINK"
            | "INCR"
            | "DECR"
            | "INCRBY"
            | "DECRBY"
            | "APPEND"
            | "EXPIRE"
            | "PEXPIRE"
            | "EXPIREAT"
            | "PEXPIREAT"
            | "PERSIST"
            | "HSET"
            | "HDEL"
            | "HINCRBY"
            | "SADD"
            | "SREM"
            | "FLUSHDB"
            | "FLUSHALL"
            | "EXEC"
    )
}

fn records_transport_request(command: &str) -> bool {
    matches!(
        command,
        "PING"
            | "ECHO"
            | "GET"
            | "SET"
            | "MGET"
            | "MSET"
            | "DEL"
            | "UNLINK"
            | "EXISTS"
            | "SCAN"
            | "KEYS"
            | "HSET"
            | "HGET"
            | "HGETALL"
            | "SADD"
            | "SMEMBERS"
            | "INFO"
            | "DBSIZE"
            | "TTL"
            | "PTTL"
            | "HELLO"
    )
}
