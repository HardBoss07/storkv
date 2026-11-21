//! Memcached storage command.

use super::super::parser::StorageKind;
use super::MemcachedSession;
use super::reply::{expiry_from_exptime, server_error, with_crlf};
use crate::compat;
use crate::compat::redis::keyspace::{self, Outcome};

impl MemcachedSession {
    pub(super) fn store(
        &mut self,
        kind: StorageKind,
        key: &[u8],
        flags: u32,
        exptime: u32,
        payload: &[u8],
        cas: Option<u64>,
    ) -> Vec<u8> {
        if let Err(response) = self.writable() {
            return response;
        }
        let scope = match self.scope() {
            Ok(scope) => scope,
            Err(response) => return response,
        };
        let expires = expiry_from_exptime(exptime);
        let db = self.db().clone();
        let result = keyspace::atomic(&db, self.server.is_primary(), |atomic| {
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
}
