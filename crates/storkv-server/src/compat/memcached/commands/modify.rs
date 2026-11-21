//! Memcached mutation and stats commands.

use super::MemcachedSession;
use super::reply::{expiry_from_exptime, server_error, with_crlf};
use crate::compat;
use crate::compat::redis::keyspace::{self, Outcome};

impl MemcachedSession {
    pub(super) fn delete(&mut self, key: &[u8]) -> Vec<u8> {
        if let Err(response) = self.writable() {
            return response;
        }
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), self.server.is_primary(), |atomic| {
            if atomic.get(&scope, key)?.is_none() {
                return Ok(Outcome::Reply(&b"NOT_FOUND"[..]));
            }
            atomic.delete(&scope, key);
            Ok(Outcome::Commit(&b"DELETED"[..]))
        }) {
            Ok(response) => with_crlf(response),
            Err(error) => server_error(error),
        }
    }

    pub(super) fn increment(&mut self, key: &[u8], delta: u64, decrement: bool) -> Vec<u8> {
        if let Err(response) = self.writable() {
            return response;
        }
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        let db = self.db().clone();
        let result = keyspace::atomic(&db, self.server.is_primary(), |atomic| {
            let Some(stored) = atomic.get(&scope, key)? else {
                return Ok(Outcome::Reply(b"NOT_FOUND".to_vec()));
            };
            let (flags, value) = compat::decode_memcached_value(&stored);
            let text = std::str::from_utf8(value).ok();
            let current = text.and_then(|text| text.parse::<u64>().ok());
            let Some(current) = current else {
                return Ok(Outcome::Reply(
                    b"CLIENT_ERROR cannot increment or decrement non-numeric value".to_vec(),
                ));
            };
            let next = if decrement {
                current.saturating_sub(delta)
            } else {
                match current.checked_add(delta) {
                    Some(value) => value,
                    None => {
                        return Ok(Outcome::Reply(
                            b"CLIENT_ERROR cannot increment or decrement non-numeric value"
                                .to_vec(),
                        ));
                    }
                }
            };
            let expires = db
                .key_meta(&scope, key)?
                .map(|meta| meta.expires_at_ms)
                .unwrap_or(0);
            let encoded = compat::encode_memcached_value(flags, next.to_string().as_bytes());
            atomic.put(&scope, key, &encoded, expires);
            Ok(Outcome::Commit(next.to_string().into_bytes()))
        });
        match result {
            Ok(response) => with_crlf(&response),
            Err(error) => server_error(error),
        }
    }

    pub(super) fn touch(&mut self, key: &[u8], exptime: u32) -> Vec<u8> {
        if let Err(response) = self.writable() {
            return response;
        }
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(self.db(), self.server.is_primary(), |atomic| {
            let Some(stored) = atomic.get(&scope, key)? else {
                return Ok(Outcome::Reply(&b"NOT_FOUND"[..]));
            };
            atomic.put(&scope, key, &stored, expiry_from_exptime(exptime));
            Ok(Outcome::Commit(&b"TOUCHED"[..]))
        }) {
            Ok(response) => with_crlf(response),
            Err(error) => server_error(error),
        }
    }

    pub(super) fn flush(&mut self) -> Vec<u8> {
        if let Err(error) = self.server.ensure_primary() {
            return server_error(error);
        }
        if self.strict_admin {
            return b"CLIENT_ERROR flush_all requires an admin connection\r\n".to_vec();
        }
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match compat::flush_scope(self.db(), &scope) {
            Ok(()) => b"OK\r\n".to_vec(),
            Err(error) => server_error(error),
        }
    }

    pub(super) fn stats(&mut self) -> Vec<u8> {
        let items = self
            .scope()
            .ok()
            .and_then(|scope| self.db().scan(&scope, None::<&[u8]>, None::<&[u8]>).ok())
            .map(|pairs| pairs.len())
            .unwrap_or(0);
        format!(
            "STAT pid 1\r\nSTAT uptime 0\r\nSTAT time 0\r\nSTAT version storkv-0.1.0\r\n\
             STAT curr_items {items}\r\nSTAT total_items {items}\r\nEND\r\n"
        )
        .into_bytes()
    }
}
