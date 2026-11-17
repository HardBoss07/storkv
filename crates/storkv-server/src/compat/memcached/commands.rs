//! Memcached ASCII command handlers.

use std::sync::Arc;

use bytes::Bytes;
use storkv_core::{Db, ScopeHandle, StorKvError};

use super::parser::{Command, StorageKind};
use crate::compat;
use crate::compat::redis::keyspace::{self, Outcome};
use crate::server::Server;

pub(crate) struct MemcachedSession {
    db: Arc<Db>,
    scope: Option<ScopeHandle>,
    pub(crate) close: bool,
}

impl MemcachedSession {
    pub(crate) fn new(server: &Arc<Server>) -> Self {
        Self {
            db: server.db().clone(),
            scope: None,
            close: false,
        }
    }

    pub(crate) fn execute(&mut self, command: Command, data: Option<Bytes>) -> Vec<u8> {
        match command {
            Command::Get { keys, with_cas } => self.get(&keys, with_cas),
            Command::Storage {
                kind,
                key,
                flags,
                exptime,
                bytes: _,
                cas,
                noreply,
            } => {
                let payload = data.unwrap_or_default();
                let response = self.store(kind, &key, flags, exptime, &payload, cas);
                silence_if(noreply, response)
            }
            Command::Delete { key, noreply } => silence_if(noreply, self.delete(&key)),
            Command::Increment {
                key,
                delta,
                decrement,
                noreply,
            } => silence_if(noreply, self.increment(&key, delta, decrement)),
            Command::Touch {
                key,
                exptime,
                noreply,
            } => silence_if(noreply, self.touch(&key, exptime)),
            Command::FlushAll { noreply, .. } => silence_if(noreply, self.flush()),
            Command::Stats => self.stats(),
            Command::Version => b"VERSION storkv-0.1.0\r\n".to_vec(),
            Command::Quit => {
                self.close = true;
                Vec::new()
            }
        }
    }

    fn scope(&mut self) -> Result<ScopeHandle, Vec<u8>> {
        if let Some(scope) = &self.scope {
            return Ok(scope.clone());
        }
        match compat::memcached_scope(&self.db) {
            Ok(scope) => {
                self.scope = Some(scope.clone());
                Ok(scope)
            }
            Err(error) => Err(server_error(error)),
        }
    }

    fn get(&mut self, keys: &[Bytes], with_cas: bool) -> Vec<u8> {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        let mut output = Vec::new();
        for key in keys {
            let Some(stored) = self.lookup(&scope, key) else {
                continue;
            };
            let (flags, value) = compat::decode_memcached_value(&stored);
            let header = if with_cas {
                let cas = self
                    .db
                    .key_meta(&scope, key)
                    .ok()
                    .flatten()
                    .map(|meta| meta.seq)
                    .unwrap_or(0);
                format!(
                    "VALUE {} {} {} {}\r\n",
                    String::from_utf8_lossy(key),
                    flags,
                    value.len(),
                    cas
                )
            } else {
                format!(
                    "VALUE {} {} {}\r\n",
                    String::from_utf8_lossy(key),
                    flags,
                    value.len()
                )
            };
            output.extend_from_slice(header.as_bytes());
            output.extend_from_slice(value);
            output.extend_from_slice(b"\r\n");
        }
        output.extend_from_slice(b"END\r\n");
        output
    }

    fn store(
        &mut self,
        kind: StorageKind,
        key: &[u8],
        flags: u32,
        exptime: u32,
        payload: &[u8],
        cas: Option<u64>,
    ) -> Vec<u8> {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        let expires = expiry_from_exptime(exptime);
        let db = self.db.clone();
        let result = keyspace::atomic(&db, |atomic| {
            let current = atomic.get(&scope, key)?;
            match kind {
                StorageKind::Add if current.is_some() => {
                    return Ok(Outcome::Reply(&b"NOT_STORED"[..]));
                }
                StorageKind::Replace if current.is_none() => {
                    return Ok(Outcome::Reply(&b"NOT_STORED"[..]));
                }
                StorageKind::Cas => {
                    let Some(token) = cas else {
                        return Ok(Outcome::Reply(&b"CLIENT_ERROR invalid cas token"[..]));
                    };
                    let Some(_) = current else {
                        return Ok(Outcome::Reply(&b"NOT_FOUND"[..]));
                    };
                    let current_token = db.key_meta(&scope, key)?.map(|meta| meta.seq).unwrap_or(0);
                    if current_token != token {
                        return Ok(Outcome::Reply(&b"EXISTS"[..]));
                    }
                }
                StorageKind::Append | StorageKind::Prepend => {
                    let Some(stored) = current else {
                        return Ok(Outcome::Reply(&b"NOT_STORED"[..]));
                    };
                    let (current_flags, value) = compat::decode_memcached_value(&stored);
                    let mut combined = Vec::with_capacity(value.len() + payload.len());
                    if kind == StorageKind::Append {
                        combined.extend_from_slice(value);
                        combined.extend_from_slice(payload);
                    } else {
                        combined.extend_from_slice(payload);
                        combined.extend_from_slice(value);
                    }
                    let expires = db
                        .key_meta(&scope, key)?
                        .map(|meta| meta.expires_at_ms)
                        .unwrap_or(0);
                    let encoded = compat::encode_memcached_value(current_flags, &combined);
                    atomic.put(&scope, key, &encoded, expires);
                    return Ok(Outcome::Commit(&b"STORED"[..]));
                }
                _ => {}
            }
            let encoded = compat::encode_memcached_value(flags, payload);
            atomic.put(&scope, key, &encoded, expires);
            Ok(Outcome::Commit(&b"STORED"[..]))
        });
        match result {
            Ok(response) => with_crlf(response),
            Err(error) => server_error(error),
        }
    }

    fn delete(&mut self, key: &[u8]) -> Vec<u8> {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(&self.db, |atomic| {
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

    fn increment(&mut self, key: &[u8], delta: u64, decrement: bool) -> Vec<u8> {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        let db = self.db.clone();
        let result = keyspace::atomic(&db, |atomic| {
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

    fn touch(&mut self, key: &[u8], exptime: u32) -> Vec<u8> {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match keyspace::atomic(&self.db, |atomic| {
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

    fn flush(&mut self) -> Vec<u8> {
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        match compat::flush_scope(&self.db, &scope) {
            Ok(_) => b"OK\r\n".to_vec(),
            Err(error) => server_error(error),
        }
    }

    fn stats(&mut self) -> Vec<u8> {
        let items = self
            .scope()
            .ok()
            .and_then(|scope| self.db.scan(&scope, None::<&[u8]>, None::<&[u8]>).ok())
            .map(|pairs| pairs.len())
            .unwrap_or(0);
        format!(
            "STAT pid 1\r\nSTAT uptime 0\r\nSTAT time 0\r\nSTAT version storkv-0.1.0\r\n\
             STAT curr_items {items}\r\nSTAT total_items {items}\r\nEND\r\n"
        )
        .into_bytes()
    }

    fn lookup(&self, scope: &ScopeHandle, key: &[u8]) -> Option<Bytes> {
        self.db.get(scope, key).ok().flatten()
    }
}

fn silence_if(noreply: bool, response: Vec<u8>) -> Vec<u8> {
    if noreply { Vec::new() } else { response }
}

fn with_crlf(response: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(response.len() + 2);
    output.extend_from_slice(response);
    output.extend_from_slice(b"\r\n");
    output
}

fn server_error(error: StorKvError) -> Vec<u8> {
    format!("SERVER_ERROR {error}\r\n").into_bytes()
}

fn expiry_from_exptime(exptime: u32) -> u64 {
    if exptime == 0 {
        return 0;
    }
    let seconds = u64::from(exptime);
    if seconds > 30 * 24 * 60 * 60 {
        seconds.saturating_mul(1000)
    } else {
        storkv_core::TimestampMs::now()
            .get()
            .saturating_add(seconds.saturating_mul(1000))
    }
}
