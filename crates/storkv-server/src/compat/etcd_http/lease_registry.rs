//! In-memory lease table for the etcd gateway.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use bytes::Bytes;

use super::now_ms;

struct Lease {
    deadline_ms: u64,
    granted_ttl_ms: u64,
    keys: Vec<Bytes>,
}

struct RegistryInner {
    next_id: u64,
    leases: HashMap<u64, Lease>,
}

/// In-memory lease table shared by every HTTP connection of one server.
pub(crate) struct LeaseRegistry {
    inner: Mutex<RegistryInner>,
}

impl Default for LeaseRegistry {
    fn default() -> Self {
        Self {
            inner: Mutex::new(RegistryInner {
                next_id: 1,
                leases: HashMap::new(),
            }),
        }
    }
}

impl LeaseRegistry {
    fn lock(&self) -> MutexGuard<'_, RegistryInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Create a lease and return its identifier.
    pub(crate) fn grant(&self, ttl_ms: u64) -> u64 {
        let mut inner = self.lock();
        let id = inner.next_id;
        inner.next_id = inner.next_id.saturating_add(1);
        inner.leases.insert(
            id,
            Lease {
                deadline_ms: now_ms().saturating_add(ttl_ms),
                granted_ttl_ms: ttl_ms,
                keys: Vec::new(),
            },
        );
        id
    }

    /// Return the absolute expiry of a live lease, or `None` when it is gone.
    pub(crate) fn expiry(&self, id: u64, now_ms: u64) -> Option<u64> {
        let mut inner = self.lock();
        match inner.leases.get(&id) {
            Some(lease) if lease.deadline_ms > now_ms => Some(lease.deadline_ms),
            Some(_) => {
                inner.leases.remove(&id);
                None
            }
            None => None,
        }
    }

    /// Return the lease that owns a key, or `0` when no lease owns it.
    pub(crate) fn lease_for(&self, key: &[u8]) -> u64 {
        let inner = self.lock();
        inner
            .leases
            .iter()
            .find(|(_, lease)| lease.keys.iter().any(|owned| owned.as_ref() == key))
            .map(|(id, _)| *id)
            .unwrap_or(0)
    }

    /// Attach a key to a lease. A `0` identifier detaches the key.
    pub(crate) fn set_key_lease(&self, key: &[u8], id: u64) {
        let mut inner = self.lock();
        for lease in inner.leases.values_mut() {
            lease.keys.retain(|owned| owned.as_ref() != key);
        }
        if let Some(lease) = inner.leases.get_mut(&id) {
            lease.keys.push(Bytes::copy_from_slice(key));
        }
    }

    /// Remove a key from every lease.
    pub(crate) fn detach(&self, key: &[u8]) {
        let mut inner = self.lock();
        for lease in inner.leases.values_mut() {
            lease.keys.retain(|owned| owned.as_ref() != key);
        }
    }

    /// Return the granted lifetime and keys of a live lease.
    pub(crate) fn snapshot(&self, id: u64, now_ms: u64) -> Option<(u64, Vec<Bytes>)> {
        let mut inner = self.lock();
        match inner.leases.get(&id) {
            Some(lease) if lease.deadline_ms > now_ms => {
                Some((lease.granted_ttl_ms, lease.keys.clone()))
            }
            Some(_) => {
                inner.leases.remove(&id);
                None
            }
            None => None,
        }
    }

    /// Move a live lease deadline forward after a keepalive.
    pub(crate) fn refresh(&self, id: u64, deadline_ms: u64) {
        if let Some(lease) = self.lock().leases.get_mut(&id) {
            lease.deadline_ms = deadline_ms;
        }
    }

    /// Remove a lease and return its attached keys.
    pub(crate) fn take(&self, id: u64) -> Option<Vec<Bytes>> {
        self.lock().leases.remove(&id).map(|lease| lease.keys)
    }

    /// Return deadline, granted lifetime, and keys of a live lease.
    pub(crate) fn status(&self, id: u64, now_ms: u64) -> Option<(u64, u64, Vec<Bytes>)> {
        let mut inner = self.lock();
        match inner.leases.get(&id) {
            Some(lease) if lease.deadline_ms > now_ms => {
                Some((lease.deadline_ms, lease.granted_ttl_ms, lease.keys.clone()))
            }
            Some(_) => {
                inner.leases.remove(&id);
                None
            }
            None => None,
        }
    }
}
