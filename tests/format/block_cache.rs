use bytes::Bytes;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use storkv_core::format::{InternalKey, ValueType};
use storkv_core::sstable::{BlockCache, BlockKey, CompressionType, SstReader, SstWriter};
use storkv_core::telemetry::Telemetry;
use storkv_core::vfs::{FileSystem, PosixFs};

fn write_test_sstable(path: &std::path::Path) {
    let fs = Arc::new(PosixFs);
    let mut file = fs.create_file(path).unwrap();
    let mut writer = SstWriter::new(&mut file, 1, 0, CompressionType::None, None);
    for index in 0..400u64 {
        let user_key = format!("key:{index:05}").into_bytes();
        let internal_key = InternalKey::new(
            Bytes::copy_from_slice(&user_key),
            index + 1,
            ValueType::Value,
            0,
        );
        let value = format!("value:{index:05}").into_bytes();
        writer
            .add(
                &internal_key.encode(),
                &user_key,
                index + 1,
                0,
                false,
                &value,
            )
            .unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn block_cache_records_hits_and_misses() {
    let path = std::env::temp_dir().join(format!(
        "storkv_block_cache_{}_{}.sst",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    write_test_sstable(&path);

    let fs = Arc::new(PosixFs);
    let telemetry = Arc::new(Telemetry::new());
    let cache = BlockCache::new(64 * 1024, telemetry.clone());
    let mut file = fs.open_file(&path).unwrap();

    let reader = SstReader::open_cached(&mut file, &cache, 1, 7, true).unwrap();
    let entries = reader
        .read_all_entries_cached(&mut file, &cache, 1, 7, true)
        .unwrap();
    assert_eq!(entries.len(), 400);
    assert!(cache.approximate_size() > 0);

    let first_hits = telemetry.block_cache_hits_total.load(Ordering::Relaxed);
    let _ = reader
        .read_all_entries_cached(&mut file, &cache, 1, 7, true)
        .unwrap();
    let second_hits = telemetry.block_cache_hits_total.load(Ordering::Relaxed);
    assert!(
        second_hits > first_hits,
        "second read must hit cached blocks"
    );
    assert!(telemetry.block_cache_misses_total.load(Ordering::Relaxed) > 0);
    assert_eq!(entries.len(), 400);

    let _ = fs.remove_file(&path);
}

#[test]
fn fill_cache_false_does_not_pollute_cache() {
    let path = std::env::temp_dir().join(format!(
        "storkv_block_cache_nofill_{}_{}.sst",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    write_test_sstable(&path);

    let fs = Arc::new(PosixFs);
    let telemetry = Arc::new(Telemetry::new());
    let cache = BlockCache::new(64 * 1024, telemetry.clone());
    let mut file = fs.open_file(&path).unwrap();

    let reader = SstReader::open_cached(&mut file, &cache, 1, 9, false).unwrap();
    let entries = reader
        .read_all_entries_cached(&mut file, &cache, 1, 9, false)
        .unwrap();
    assert_eq!(entries.len(), 400);
    assert_eq!(cache.approximate_size(), 0);
    assert_eq!(telemetry.block_cache_hits_total.load(Ordering::Relaxed), 0);
    assert!(telemetry.block_cache_misses_total.load(Ordering::Relaxed) > 0);

    let _ = fs.remove_file(&path);
}

const CACHE_SHARDS: usize = 16;
const CACHE_VALUE_LEN: usize = 64;

fn cache_value() -> Bytes {
    Bytes::from(vec![7u8; CACHE_VALUE_LEN])
}

/// Find keys that hash into the same shard by using a one-entry-per-shard
/// budget: a same-shard insert evicts the previous key.
fn keys_sharing_one_shard(count: usize) -> Vec<BlockKey> {
    let mut keys = Vec::with_capacity(count);
    let mut candidate = 0u64;
    while keys.len() < count {
        let key = BlockKey::new(1, candidate, 0);
        candidate += 1;
        if keys.is_empty() {
            keys.push(key);
            continue;
        }
        let cache = BlockCache::new(CACHE_SHARDS * CACHE_VALUE_LEN, Arc::new(Telemetry::new()));
        for cached in &keys {
            cache.put(*cached, cache_value());
        }
        cache.put(key, cache_value());
        let last = *keys.last().expect("keys is not empty");
        if cache.get(&last).is_none() {
            keys.push(key);
        }
    }
    keys
}

#[test]
fn block_cache_evicts_least_recently_used_block() {
    let keys = keys_sharing_one_shard(3);
    let cache = BlockCache::new(
        CACHE_SHARDS * 2 * CACHE_VALUE_LEN,
        Arc::new(Telemetry::new()),
    );
    cache.put(keys[0], cache_value());
    cache.put(keys[1], cache_value());
    assert!(cache.get(&keys[0]).is_some(), "both blocks fit the shard");

    cache.put(keys[2], cache_value());
    assert!(
        cache.get(&keys[0]).is_some(),
        "a recently read block must survive eviction"
    );
    assert!(
        cache.get(&keys[1]).is_none(),
        "the least recently used block must be evicted"
    );
    assert!(cache.get(&keys[2]).is_some());
    assert!(cache.approximate_size() <= CACHE_SHARDS * 2 * CACHE_VALUE_LEN);
}

#[test]
fn zero_budget_cache_stays_empty_and_records_misses() {
    let telemetry = Arc::new(Telemetry::new());
    let cache = BlockCache::new(0, telemetry.clone());
    let key = BlockKey::new(1, 1, 0);

    cache.put(key, cache_value());
    assert_eq!(cache.approximate_size(), 0);
    assert!(cache.get(&key).is_none());
    assert_eq!(
        telemetry.block_cache_misses_total.load(Ordering::Relaxed),
        1
    );
    assert_eq!(telemetry.block_cache_hits_total.load(Ordering::Relaxed), 0);
}
