//! LRU storage with per-entry time to live.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;

use super::ScopeCache;

pub(super) struct CacheEntry {
    value: Option<Bytes>,
    expires_at: Option<Instant>,
}

pub(crate) struct CacheState {
    map: HashMap<Bytes, CacheEntry>,
    order: VecDeque<Bytes>,
}

impl CacheState {
    pub(super) fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn touch(&mut self, key: &Bytes) {
        self.order.retain(|existing| existing != key);
        self.order.push_back(key.clone());
    }

    pub(super) fn remove(&mut self, key: &Bytes) {
        self.map.remove(key);
        self.order.retain(|existing| existing != key);
    }

    fn insert(
        &mut self,
        key: Bytes,
        value: Option<Bytes>,
        expires_at: Option<Instant>,
        capacity: usize,
    ) {
        if capacity == 0 {
            return;
        }
        self.map
            .insert(key.clone(), CacheEntry { value, expires_at });
        self.touch(&key);
        while self.map.len() > capacity {
            let Some(evicted) = self.order.pop_front() else {
                break;
            };
            self.map.remove(&evicted);
        }
    }

    pub(super) fn lookup_entry(&mut self, key: &Bytes) -> Option<Option<Bytes>> {
        if self
            .map
            .get(key)
            .and_then(|entry| entry.expires_at)
            .is_some_and(|deadline| deadline <= Instant::now())
        {
            self.remove(key);
            return None;
        }
        let value = self.map.get(key).map(|entry| entry.value.clone())?;
        self.touch(key);
        Some(value)
    }

    pub(super) fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }
}

impl ScopeCache {
    pub(crate) fn lookup(&self, key: &[u8]) -> Option<Option<Bytes>> {
        let key = Bytes::copy_from_slice(key);
        self.state.lock().lookup_entry(&key)
    }

    pub(crate) fn store(&self, key: &[u8], value: Option<Bytes>) {
        let key = Bytes::copy_from_slice(key);
        self.state.lock().insert(key, value, None, self.capacity);
    }

    pub(crate) fn store_with_ttl(&self, key: &[u8], value: Option<Bytes>, ttl: Duration) {
        let key = Bytes::copy_from_slice(key);
        let expires_at = Instant::now().checked_add(ttl);
        self.state
            .lock()
            .insert(key, value, expires_at, self.capacity);
    }

    pub(crate) fn store_with_expires_at_ms(
        &self,
        key: &[u8],
        value: Option<Bytes>,
        expires_at_ms: u64,
    ) {
        let key = Bytes::copy_from_slice(key);
        let expires_at = deadline_from_epoch_ms(expires_at_ms);
        self.state
            .lock()
            .insert(key, value, expires_at, self.capacity);
    }

    pub(crate) fn invalidate(&self, key: &[u8]) {
        if !self.enabled {
            return;
        }
        let key = Bytes::copy_from_slice(key);
        self.state.lock().remove(&key);
    }

    pub(crate) fn clear(&self) {
        self.state.lock().clear();
    }
}

fn deadline_from_epoch_ms(expires_at_ms: u64) -> Option<Instant> {
    if expires_at_ms == 0 {
        return None;
    }
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(u64::MAX);
    let remaining = Duration::from_millis(expires_at_ms.saturating_sub(now_ms));
    Some(
        Instant::now()
            .checked_add(remaining)
            .unwrap_or_else(Instant::now),
    )
}
