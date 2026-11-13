use crate::format::{InternalKey, ValueType};
use bytes::Bytes;
use crossbeam_skiplist::SkipMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

pub struct MemTable {
    map: SkipMap<Bytes, Bytes>,
    approx_bytes: AtomicUsize,
    created_at: Instant,
    oldest_seq: parking_lot::RwLock<Option<u64>>,
}

impl MemTable {
    pub fn new() -> Self {
        Self {
            map: SkipMap::new(),
            approx_bytes: AtomicUsize::new(0),
            created_at: Instant::now(),
            oldest_seq: parking_lot::RwLock::new(None),
        }
    }

    pub fn insert(&self, encoded_internal_key: Bytes, value: Bytes, seq: u64) {
        let entry_size = encoded_internal_key.len() + value.len() + 32;
        self.approx_bytes.fetch_add(entry_size, Ordering::Relaxed);

        let mut oldest = self.oldest_seq.write();
        if oldest.is_none() || Some(seq) < *oldest {
            *oldest = Some(seq);
        }

        self.map.insert(encoded_internal_key, value);
    }

    pub fn size_bytes(&self) -> usize {
        self.approx_bytes.load(Ordering::Relaxed)
    }

    pub fn created_at(&self) -> Instant {
        self.created_at
    }

    pub fn oldest_seq(&self) -> Option<u64> {
        *self.oldest_seq.read()
    }

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

    pub fn range_from<'a>(
        &'a self,
        start_bound: Bytes,
    ) -> impl Iterator<Item = (Bytes, Bytes)> + 'a {
        self.map
            .range(start_bound..)
            .map(|entry| (entry.key().clone(), entry.value().clone()))
    }

    pub fn iter(&self) -> impl Iterator<Item = (Bytes, Bytes)> + '_ {
        self.map
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
    }
}

pub struct FlushQueue {
    queue: parking_lot::RwLock<Vec<Arc<MemTable>>>,
}

impl FlushQueue {
    pub fn new() -> Self {
        Self {
            queue: parking_lot::RwLock::new(Vec::new()),
        }
    }

    pub fn push(&self, mem: Arc<MemTable>) {
        self.queue.write().push(mem);
    }

    pub fn pop_oldest(&self) -> Option<Arc<MemTable>> {
        let mut q = self.queue.write();
        if q.is_empty() {
            None
        } else {
            Some(q.remove(0))
        }
    }

    pub fn oldest(&self) -> Option<Arc<MemTable>> {
        self.queue.read().first().cloned()
    }

    pub fn len(&self) -> usize {
        self.queue.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.read().is_empty()
    }

    pub fn all(&self) -> Vec<Arc<MemTable>> {
        self.queue.read().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memtable_mvcc_and_ttl() {
        let mem = MemTable::new();
        let k = b"session:user1";

        let k_v1 = InternalKey::new(Bytes::copy_from_slice(k), 10, ValueType::Value, 0).encode();
        mem.insert(k_v1, Bytes::from_static(b"v1"), 10);

        let k_v2 = InternalKey::new(Bytes::copy_from_slice(k), 20, ValueType::Value, 0).encode();
        mem.insert(k_v2, Bytes::from_static(b"v2"), 20);

        let now_ms = 1000;
        // Snapshot at seq 15 reads v1
        let (vt, val) = mem.get(k, 15, now_ms).unwrap();
        assert_eq!(vt, ValueType::Value);
        assert_eq!(val.as_ref(), b"v1");

        // Snapshot at seq 25 reads v2
        let (_vt, val) = mem.get(k, 25, now_ms).unwrap();
        assert_eq!(val.as_ref(), b"v2");

        // Tombstone at seq 30
        let k_del =
            InternalKey::new(Bytes::copy_from_slice(k), 30, ValueType::Tombstone, 0).encode();
        mem.insert(k_del, Bytes::new(), 30);

        let (vt, _) = mem.get(k, 35, now_ms).unwrap();
        assert_eq!(vt, ValueType::Tombstone);

        // Expired TTL check
        let k_exp = InternalKey::new(Bytes::copy_from_slice(k), 40, ValueType::Value, 500).encode();
        mem.insert(k_exp, Bytes::from_static(b"v4"), 40);
        assert!(mem.get(k, 45, 1000).is_none());
        assert!(mem.get(k, 45, 400).is_some());
    }
}
