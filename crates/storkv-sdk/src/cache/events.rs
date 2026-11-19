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
            ChangeEvent::Put { key, value, .. } => self.store_event(key, Some(value.clone())),
            ChangeEvent::Delete { key, .. } => self.store(key, None),
            ChangeEvent::DropScope { .. } | ChangeEvent::InvalidateScope { .. } => self.clear(),
            _ => {}
        }
    }

    pub(crate) fn store_event(&self, key: &[u8], value: Option<Bytes>) {
        let key = Bytes::copy_from_slice(key);
        self.state.lock().preserve_expiry(key, value, self.capacity);
    }
}
