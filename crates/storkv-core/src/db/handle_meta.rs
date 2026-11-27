//! Key metadata and TTL inspection helpers.

use std::sync::atomic::Ordering;

use super::{Db, ScopeHandle};

impl Db {
    /// Return the un-flushed WAL bytes and the total on-disk WAL bytes.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the WAL directory cannot be read.
    pub fn wal_byte_stats(&self) -> crate::Result<(u64, u64)> {
        self.inner.wal_byte_stats()
    }

    /// Return the number of live watch subscribers.
    pub fn watch_subscriber_count(&self) -> usize {
        self.inner.watch_registry.subscriber_count()
    }

    /// Metadata (absolute expiry and sequence) of the visible version of a key.
    pub fn key_meta(
        &self,
        scope: &ScopeHandle,
        key: impl AsRef<[u8]>,
    ) -> crate::Result<Option<crate::scope::KeyMeta>> {
        let read_seq = self.inner.last_visible_seq.load(Ordering::Acquire);
        let scope_state = self
            .inner
            .scopes
            .read()
            .get(&scope.scope_id)
            .cloned()
            .ok_or(crate::StorkvError::ScopeIdNotFound(scope.scope_id))?;
        scope_state.key_meta(
            key.as_ref(),
            crate::types::SequenceNumber::new(read_seq),
            crate::types::TimestampMs::now(),
        )
    }

    /// Remaining TTL in milliseconds.
    ///
    /// `None` means the key does not exist. `Some(0)` means the key is
    /// permanent; any other value is the exact remaining lifetime.
    pub fn key_ttl_remaining_ms(
        &self,
        scope: &ScopeHandle,
        key: impl AsRef<[u8]>,
    ) -> crate::Result<Option<u64>> {
        let Some(meta) = self.key_meta(scope, key)? else {
            return Ok(None);
        };
        if meta.expires_at_ms == 0 {
            return Ok(Some(0));
        }
        let now_ms = crate::types::TimestampMs::now().get();
        Ok(Some(meta.expires_at_ms.saturating_sub(now_ms)))
    }

    /// Run one synchronous pass across scopes with active TTL tracking,
    /// expiring due keys and emitting events.
    ///
    /// # Errors
    ///
    /// Returns an error when writing an expiration tombstone fails.
    pub fn expire_due_keys(&self) -> crate::Result<usize> {
        self.inner.expire_due_keys()
    }
}
