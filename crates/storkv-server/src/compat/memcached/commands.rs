//! Memcached ASCII command handlers.

use std::sync::Arc;

use bytes::Bytes;
use storkv_core::{Db, ScopeHandle};

use super::parser::Command;
use crate::compat;
use crate::server::Server;

mod modify;
mod reply;
mod store;

use reply::silence_if;

pub(crate) struct MemcachedSession {
    pub(crate) close: bool,
    strict_admin: bool,
    db: Arc<Db>,
    scope: Option<ScopeHandle>,
}

impl MemcachedSession {
    pub(crate) fn new(server: &Arc<Server>) -> Self {
        Self {
            close: false,
            strict_admin: server.strict_admin(),
            db: server.db().clone(),
            scope: None,
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
            Err(error) => Err(reply::server_error(error)),
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

    fn lookup(&self, scope: &ScopeHandle, key: &[u8]) -> Option<Bytes> {
        self.db.get(scope, key).ok().flatten()
    }
}
