//! Bounded per-scope read cache with watch-based invalidation.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use parking_lot::Mutex;

use crate::options::CacheOptions;

mod events;
mod store;

use store::CacheState;

/// Bounded per-scope read cache with watch-based invalidation.
pub(crate) struct ScopeCache {
    pub(super) enabled: bool,
    pub(super) watcher_started: AtomicBool,
    pub(super) capacity: usize,
    pub(super) state: Mutex<CacheState>,
}

impl ScopeCache {
    pub(crate) fn new(options: CacheOptions) -> Arc<Self> {
        Arc::new(Self {
            enabled: options.enabled,
            watcher_started: AtomicBool::new(false),
            capacity: options.max_entries_per_scope,
            state: Mutex::new(CacheState::new()),
        })
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }
}
