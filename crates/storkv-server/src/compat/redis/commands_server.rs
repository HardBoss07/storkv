//! Connection-level Redis commands: PING, HELLO, SELECT, INFO, and flushing.

use bytes::Bytes;

use super::RedisSession;
use super::codec::Resp;
use super::helpers::{storage_error, wrong_arity};
use crate::compat;

impl RedisSession {
    pub(super) fn ping(&self, args: &[Bytes]) -> Resp {
        match args.get(1) {
            Some(message) => Resp::bulk(message.clone()),
            None => Resp::simple("PONG"),
        }
    }

    pub(super) fn echo(&self, args: &[Bytes]) -> Resp {
        match args.get(1) {
            Some(message) => Resp::bulk(message.clone()),
            None => wrong_arity("echo"),
        }
    }

    pub(super) fn hello(&mut self, args: &[Bytes]) -> Resp {
        if let Some(version) = args.get(1) {
            match version.as_ref() {
                b"2" => self.resp_version = 2,
                b"3" => self.resp_version = 3,
                other => {
                    return Resp::error(format!(
                        "NOPROTO unsupported protocol version {}",
                        String::from_utf8_lossy(other)
                    ));
                }
            }
        }
        Resp::Map(vec![
            (Resp::bulk("server"), Resp::bulk("storkv")),
            (Resp::bulk("version"), Resp::bulk(env!("CARGO_PKG_VERSION"))),
            (
                Resp::bulk("proto"),
                Resp::Integer(i64::from(self.resp_version)),
            ),
            (Resp::bulk("id"), Resp::Integer(1)),
            (Resp::bulk("mode"), Resp::bulk("standalone")),
            (Resp::bulk("role"), Resp::bulk("master")),
            (Resp::bulk("modules"), Resp::array(Vec::new())),
        ])
    }

    pub(super) fn select(&mut self, args: &[Bytes]) -> Resp {
        let Some(database) = args.get(1).and_then(|arg| super::helpers::parse_u64(arg)) else {
            return Resp::error("ERR invalid DB index");
        };
        let Some(database) = u32::try_from(database).ok() else {
            return Resp::error("ERR DB index is out of range");
        };
        let previous = self.database;
        self.database = database;
        match compat::redis_scope(self.db(), database) {
            Ok(_) => Resp::simple("OK"),
            Err(error) => {
                self.database = previous;
                storage_error(error)
            }
        }
    }

    pub(super) fn info(&self) -> Resp {
        use std::sync::atomic::Ordering::Relaxed;
        let telemetry = self.db().telemetry();
        let text = format!(
            "# Server\r\nredis_version:7.2.0\r\nstorkv_version:{}\r\nredis_mode:standalone\r\n\
             # Stats\r\ntotal_writes:{}\r\ntotal_reads:{}\r\nwrite_conflicts:{}\r\n\
             # Persistence\r\ncompactions:{}\r\nfsyncs:{}\r\n",
            env!("CARGO_PKG_VERSION"),
            telemetry.writes_total.load(Relaxed),
            telemetry.reads_total.load(Relaxed),
            telemetry.write_conflicts_total.load(Relaxed),
            telemetry.compactions_total.load(Relaxed),
            telemetry.fsyncs_total.load(Relaxed),
        );
        Resp::bulk(text)
    }

    pub(super) fn dbsize(&self) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match self.db().scan(&scope, None::<&[u8]>, None::<&[u8]>) {
            Ok(pairs) => Resp::Integer(pairs.len() as i64),
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn flush_db(&self) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match compat::flush_scope(self.db(), &scope) {
            Ok(_) => Resp::simple("OK"),
            Err(error) => storage_error(error),
        }
    }

    pub(super) fn flush_all(&self) -> Resp {
        match compat::flush_redis_scopes(self.db()) {
            Ok(_) => Resp::simple("OK"),
            Err(error) => storage_error(error),
        }
    }
}
