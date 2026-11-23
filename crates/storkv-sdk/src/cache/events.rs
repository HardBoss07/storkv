//! Watch-event invalidation for the per-scope cache.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bytes::Bytes;
use storkv_core::watch::ChangeEvent;

use crate::pool::{Backend, StorkvPool};
use crate::watch::{spawn_embedded_cache_watch, spawn_networked_cache_watch};

use super::ScopeCache;

impl ScopeCache {
    pub(crate) fn ensure_watcher(self: &Arc<Self>, pool: &StorkvPool, scope_id: u32) {
        if !self.enabled || self.watcher_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(self);
        match pool.inner_backend() {
            Backend::Embedded(db) => spawn_embedded_cache_watch(db.clone(), scope_id, weak),
            Backend::Networked(backend) => {
                spawn_networked_cache_watch(backend.uri.clone(), scope_id, weak);
            }
        }
    }

    pub(crate) fn apply_event(&self, event: &ChangeEvent, scope_id: u32) {
        if event.scope_id() != scope_id {
            return;
        }
        match event {
            ChangeEvent::Put {
                key,
                value,
                expires_at_ms,
                ..
            } => self.store_event(key, Some(value.clone()), *expires_at_ms),
            ChangeEvent::Delete { key, .. } | ChangeEvent::Expired { key, .. } => {
                self.store(key, None)
            }
            ChangeEvent::DropScope { .. } | ChangeEvent::InvalidateScope { .. } => self.clear(),
            _ => {}
        }
    }

    pub(crate) fn store_event(&self, key: &[u8], value: Option<Bytes>, expires_at_ms: u64) {
        self.store_with_expires_at_ms(key, value, expires_at_ms);
    }
}
