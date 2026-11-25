//! Redis `MULTI`/`EXEC`/`WATCH` implemented on one MVCC transaction.

use bytes::Bytes;
use storkv_core::{StorkvError, Transaction};

use super::codec::Resp;
use super::commands::{MILLIS_PER_SECOND, RedisSession, WatchState};
use super::helpers::{storage_error, upper, wrong_arity};

mod counters;
mod expiry;
mod scan;
mod strings;

use counters::{tx_counter, tx_counter_by};
use expiry::{tx_expire, tx_persist, tx_ttl};
use scan::{tx_keys, tx_scan};
use strings::{
    tx_append, tx_delete, tx_exists, tx_get, tx_mget, tx_mset, tx_set, tx_setex, tx_setnx,
    tx_strlen,
};

impl RedisSession {
    pub(super) fn watch(&mut self, args: &[Bytes]) -> Resp {
        if self.multi.is_some() {
            return Resp::error("ERR WATCH inside MULTI is not allowed");
        }
        if args.len() < 2 {
            return wrong_arity("watch");
        }
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        let read_seq = self
            .db()
            .inner()
            .last_visible_seq
            .load(std::sync::atomic::Ordering::Acquire);
        let state = self.watch.get_or_insert_with(|| WatchState {
            read_seq,
            keys: Vec::new(),
        });
        for key in &args[1..] {
            state.keys.push((scope.scope_id, key.clone()));
        }
        Resp::simple("OK")
    }

    pub(super) fn exec(&mut self) -> Resp {
        if self.server.ensure_primary().is_err() {
            return super::helpers::readonly_error();
        }
        let Some(queue) = self.multi.take() else {
            return Resp::error("ERR EXEC without MULTI");
        };
        let had_error = std::mem::replace(&mut self.multi_error, false);
        let watch = self.watch.take();
        if had_error {
            return Resp::error("EXECABORT Transaction discarded because of previous errors.");
        }
        let db = self.server.db().clone();
        let mut transaction = match &watch {
            Some(state) => db.begin_tx_at(state.read_seq),
            None => db.begin_tx(),
        };
        if let Some(state) = &watch {
            for (scope_id, key) in &state.keys {
                transaction.record_read(*scope_id, key);
            }
        }
        let mut results = Vec::with_capacity(queue.len());
        for args in queue {
            let command = args.first().map(|bytes| upper(bytes)).unwrap_or_default();
            results.push(self.execute_in_tx(&mut transaction, &command, &args));
        }
        match transaction.commit() {
            Ok(()) => Resp::array(results),
            Err(StorkvError::WriteConflict { .. }) => Resp::Array(None),
            Err(error) => storage_error(error),
        }
    }

    fn execute_in_tx(
        &mut self,
        transaction: &mut Transaction,
        command: &str,
        args: &[Bytes],
    ) -> Resp {
        match command {
            "PING" | "ECHO" | "HELLO" | "SELECT" | "INFO" | "DBSIZE" | "AUTH" | "CLIENT"
            | "COMMAND" => self.execute_now(command, args),
            "UNWATCH" => {
                self.watch = None;
                Resp::simple("OK")
            }
            "FLUSHDB" | "FLUSHALL" => Resp::error("ERR command is not allowed inside MULTI"),
            _ => self.execute_tx_data(transaction, command, args),
        }
    }

    fn execute_tx_data(&mut self, tx: &mut Transaction, command: &str, args: &[Bytes]) -> Resp {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match command {
            "GET" => tx_get(tx, &scope, args),
            "SET" => tx_set(self, tx, &scope, args),
            "SETEX" => tx_setex(tx, &scope, args, MILLIS_PER_SECOND),
            "PSETEX" => tx_setex(tx, &scope, args, 1),
            "SETNX" => tx_setnx(tx, &scope, args),
            "MGET" => tx_mget(tx, &scope, args),
            "MSET" => tx_mset(tx, &scope, args),
            "DEL" | "UNLINK" => tx_delete(tx, &scope, args),
            "EXISTS" => tx_exists(tx, &scope, args),
            "INCR" => tx_counter(self, tx, &scope, args, 1),
            "DECR" => tx_counter(self, tx, &scope, args, -1),
            "INCRBY" => tx_counter_by(self, tx, &scope, args, 1),
            "DECRBY" => tx_counter_by(self, tx, &scope, args, -1),
            "APPEND" => tx_append(self, tx, &scope, args),
            "STRLEN" => tx_strlen(tx, &scope, args),
            "EXPIRE" => tx_expire(tx, &scope, args, MILLIS_PER_SECOND),
            "PEXPIRE" => tx_expire(tx, &scope, args, 1),
            "PERSIST" => tx_persist(tx, &scope, args),
            "TTL" => tx_ttl(self, &scope, args, MILLIS_PER_SECOND),
            "PTTL" => tx_ttl(self, &scope, args, 1),
            "KEYS" => tx_keys(tx, &scope, args),
            "SCAN" => tx_scan(tx, &scope, args),
            other => Resp::error(format!("ERR unknown command '{other}'")),
        }
    }
}
