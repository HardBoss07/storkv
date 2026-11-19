use crate::format::{InternalKey, ValueType};
use bytes::Bytes;
use crossbeam_skiplist::SkipMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Concurrent in-memory write buffer ordered by internal key.
pub struct MemTable {
    created_at: Instant,
    map: SkipMap<Bytes, Bytes>,
    approx_bytes: AtomicUsize,
    oldest_seq: parking_lot::RwLock<Option<u64>>,
}

impl MemTable {
    /// Create an empty memtable.
    pub fn new() -> Self {
        Self {
            map: SkipMap::new(),
            approx_bytes: AtomicUsize::new(0),
            created_at: Instant::now(),
            oldest_seq: parking_lot::RwLock::new(None),
        }
    }

    /// Insert an encoded internal key with its value and commit sequence.
    pub fn insert(&self, encoded_internal_key: Bytes, value: Bytes, seq: u64) {
        let entry_size = encoded_internal_key.len() + value.len() + 32;
        self.approx_bytes.fetch_add(entry_size, Ordering::Relaxed);

        let mut oldest = self.oldest_seq.write();
        if oldest.is_none() || Some(seq) < *oldest {
            *oldest = Some(seq);
        }

        self.map.insert(encoded_internal_key, value);
    }

    /// Return the approximate memory use in bytes.
    pub fn size_bytes(&self) -> usize {
        self.approx_bytes.load(Ordering::Relaxed)
    }

    /// Return the monotonic creation time.
    pub fn created_at(&self) -> Instant {
        self.created_at
    }

    /// Return the oldest commit sequence in the memtable.
    pub fn oldest_seq(&self) -> Option<u64> {
        *self.oldest_seq.read()
    }

    /// Return true when the memtable holds no entries.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Point lookup for the latest version with seq <= read_seq
    pub fn get(&self, user_key: &[u8], read_seq: u64, now_ms: u64) -> Option<(ValueType, Bytes)> {
        // Range search starting from user_key
        let dummy_start = InternalKey::new(
            Bytes::copy_from_slice(user_key),
            u64::MAX,
            ValueType::Value,
            0,
        )
        .encode();

        for entry in self.map.range(dummy_start..) {
            let (enc_k, val) = (entry.key(), entry.value());
            let ikey = InternalKey::decode(enc_k.clone())?;
            if ikey.user_key.as_ref() != user_key {
                break;
            }
            if ikey.seq <= read_seq {
                if ikey.expires_at_ms != 0 && ikey.expires_at_ms <= now_ms {
                    return None; // Expired
                }
                return Some((ikey.value_type, val.clone()));
            }
        }
        None
    }

    /// Return entries at or after the encoded start key.
    /// Return entries at or after the encoded start key.
    pub fn range_from<'a>(
        &'a self,
        start_bound: Bytes,
    ) -> impl Iterator<Item = (Bytes, Bytes)> + 'a {
        self.map
            .range(start_bound..)
            .map(|entry| (entry.key().clone(), entry.value().clone()))
    }

    /// Iterate every entry in internal key order.
    pub fn iter(&self) -> impl Iterator<Item = (Bytes, Bytes)> + '_ {
        self.map
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
    }
}

/// Ordered queue of immutable memtables that wait for flush.
pub struct FlushQueue {
    queue: parking_lot::RwLock<Vec<Arc<MemTable>>>,
}

impl FlushQueue {
    /// Create an empty queue.
    pub fn new() -> Self {
        Self {
            queue: parking_lot::RwLock::new(Vec::new()),
        }
    }

    /// Append a memtable to the queue.
    pub fn push(&self, mem: Arc<MemTable>) {
        self.queue.write().push(mem);
    }

    /// Remove and return the oldest queued memtable.
    pub fn pop_oldest(&self) -> Option<Arc<MemTable>> {
        let mut q = self.queue.write();
        if q.is_empty() {
            None
        } else {
            Some(q.remove(0))
        }
    }

    /// Return the oldest queued memtable without removing it.
    pub fn oldest(&self) -> Option<Arc<MemTable>> {
        self.queue.read().first().cloned()
    }

    /// Return the number of queued memtables.
    pub fn len(&self) -> usize {
        self.queue.read().len()
    }

    /// Return true when the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.read().is_empty()
    }

    /// Return a copy of every queued memtable.
    pub fn all(&self) -> Vec<Arc<MemTable>> {
        self.queue.read().clone()
    }
}
