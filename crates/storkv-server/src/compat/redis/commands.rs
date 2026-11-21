//! Redis command dispatch and per-connection session state.

use std::sync::Arc;

use bytes::Bytes;
use storkv_core::{Db, ScopeHandle};

use super::codec::Resp;
use super::helpers::{is_known_command, readonly_error, storage_error, upper};
use crate::compat;
use crate::server::Server;

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
    pub(crate) close: bool,
}

const _: () = assert!(std::mem::size_of::<RedisSession>() <= 96);

impl RedisSession {
    pub(crate) fn new(server: Arc<Server>) -> Self {
        Self {
            server,
            database: 0,
            resp_version: 2,
            multi: None,
            multi_error: false,
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
            "SET" => self.set(args),
            "SETEX" => self.setex(args, 1000),
            "PSETEX" => self.setex(args, 1),
            "SETNX" => self.setnx(args),
            "MGET" => self.mget(args),
            "MSET" => self.mset(args),
            "DEL" | "UNLINK" => self.delete(args),
            "EXISTS" => self.exists(args),
            "INCR" => self.counter(args, 1),
            "DECR" => self.counter(args, -1),
            "INCRBY" => self.counter_by(args, 1),
            "DECRBY" => self.counter_by(args, -1),
            "APPEND" => self.append(args),
            "STRLEN" => self.strlen(args),
            "EXPIRE" => self.expire(args, 1000),
            "PEXPIRE" => self.expire(args, 1),
            "PERSIST" => self.persist(args),
            "TTL" => self.ttl(args, 1000),
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

fn is_write_command(command: &str) -> bool {
    matches!(
        command,
        "SET"
            | "SETEX"
            | "PSETEX"
            | "SETNX"
            | "MSET"
            | "DEL"
            | "UNLINK"
            | "INCR"
            | "DECR"
            | "INCRBY"
            | "DECRBY"
            | "APPEND"
            | "EXPIRE"
            | "PEXPIRE"
            | "PERSIST"
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
            | "INFO"
            | "DBSIZE"
            | "TTL"
            | "PTTL"
            | "HELLO"
    )
}
