//! Sharded bounded LRU cache for SSTable data and index blocks.
//!
//! The cache is owned by one database and keyed by `(scope_id, sst_id,
//! block_offset)`. It stores decompressed block bytes so repeated reads avoid a
//! disk read and decompression. Each shard uses an independent LRU ordering and
//! byte budget to reduce lock contention between readers.

mod shard;

use shard::Shard;

use crate::telemetry::Telemetry;
use bytes::Bytes;
use parking_lot::Mutex;
use std::sync::Arc;

/// Number of independent cache shards.
const BLOCK_CACHE_SHARDS: usize = 16;
/// Hash mix constant for the scope identifier.
const HASH_MIX_SCOPE: u64 = 0x9e37_79b9_7f4a_7c15;
/// Hash mix constant for the SSTable identifier.
const HASH_MIX_SST: u64 = 0xbf58_476d_1ce4_e5b9;
/// Hash mix constant for the block offset.
const HASH_MIX_OFFSET: u64 = 0x94d0_49bb_1331_11eb;

/// Key of one cached SSTable block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockKey {
    /// Identifier of the owning scope.
    pub scope_id: u32,
    /// File number of the SSTable.
    pub sst_id: u64,
    /// Byte offset of the block in the SSTable file.
    pub block_offset: u64,
}

impl BlockKey {
    /// Create a cache key.
    pub fn new(scope_id: u32, sst_id: u64, block_offset: u64) -> Self {
        Self {
            scope_id,
            sst_id,
            block_offset,
        }
    }
}

/// Sharded bounded LRU cache for SSTable blocks.
pub struct BlockCache {
    shards: Vec<Mutex<Shard>>,
    telemetry: Arc<Telemetry>,
    max_bytes: usize,
}

impl BlockCache {
    /// Create a cache with the given total byte budget.
    ///
    /// A budget of zero disables storage and every lookup misses.
    pub fn new(max_bytes: usize, telemetry: Arc<Telemetry>) -> Self {
        let shard_bytes = max_bytes / BLOCK_CACHE_SHARDS;
        let shards = (0..BLOCK_CACHE_SHARDS)
            .map(|_| Mutex::new(Shard::new(shard_bytes)))
            .collect();
        Self {
            shards,
            telemetry,
            max_bytes,
        }
    }

    fn shard_index(key: &BlockKey) -> usize {
        let mixed = (key.scope_id as u64)
            .wrapping_mul(HASH_MIX_SCOPE)
            .wrapping_add(key.sst_id.wrapping_mul(HASH_MIX_SST))
            .wrapping_add(key.block_offset.wrapping_mul(HASH_MIX_OFFSET));
        (mixed as usize) % BLOCK_CACHE_SHARDS
    }

    /// Look up one block and record a hit or miss in telemetry.
    pub fn get(&self, key: &BlockKey) -> Option<Bytes> {
        if self.max_bytes == 0 {
            self.telemetry.record_block_cache_miss();
            return None;
        }
        let value = self.shards[Self::shard_index(key)].lock().get(key);
        if value.is_some() {
            self.telemetry.record_block_cache_hit();
        } else {
            self.telemetry.record_block_cache_miss();
        }
        value
    }

    /// Insert one block, evicting the least recently used entries as needed.
    pub fn put(&self, key: BlockKey, value: Bytes) {
        if self.max_bytes == 0 {
            return;
        }
        self.shards[Self::shard_index(&key)].lock().put(key, value);
    }

    /// Remove every cached block that belongs to one SSTable.
    pub fn invalidate_sst(&self, scope_id: u32, sst_id: u64) {
        for shard in &self.shards {
            shard.lock().invalidate_sst(scope_id, sst_id);
        }
    }

    /// Return the approximate number of bytes stored in the cache.
    pub fn approximate_size(&self) -> usize {
        self.shards.iter().map(|shard| shard.lock().bytes).sum()
    }
}
