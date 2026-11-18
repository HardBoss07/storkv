use crate::common;
use bytes::Bytes;
use storkv_core::ScopeOptions;
use storkv_core::sstable::{
    BloomFilterBuilder, BloomFilterReader, CompressionType, DataBlockReader, IndexBlockReader,
    compress_and_frame_block, decompress_and_verify_block,
};

#[test]
fn test_zero_length_keys_and_values_roundtrip() {
    let temp = common::TempDb::new("zero_len");
    let db = temp.db();
    let scope = db.create_scope("zero", ScopeOptions::default()).unwrap();

    db.put(&scope, b"", b"").unwrap();
    assert_eq!(db.get(&scope, b"").unwrap(), Some(Bytes::new()));
    assert_eq!(db.scan_prefix(&scope, b"").unwrap().len(), 1);

    db.put(&scope, b"a", b"1").unwrap();
    assert_eq!(db.scan_prefix(&scope, b"").unwrap().len(), 2);

    db.delete(&scope, b"").unwrap();
    assert_eq!(db.get(&scope, b"").unwrap(), None);
}

#[test]
fn binary_keys_and_multi_megabyte_values_survive_flush_and_reopen() {
    let mut temp = common::TempDb::new("binary_large");
    let db = temp.db();
    let scope = db.create_scope("binary", ScopeOptions::default()).unwrap();

    let key = b"bin\0key\xff".as_slice();
    let prefix_key = b"bin\0other".as_slice();
    let large_value: Vec<u8> = (0..3 * 1024 * 1024)
        .map(|index| (index % 251) as u8)
        .collect();

    db.put(&scope, key, &large_value).unwrap();
    db.put(&scope, prefix_key, b"small").unwrap();
    let scope_arc = db
        .inner()
        .scopes
        .read()
        .get(&scope.scope_id)
        .cloned()
        .unwrap();
    db.inner().flush_scope_memtable(&scope_arc).unwrap();

    assert_eq!(
        db.get(&scope, key).unwrap().as_deref(),
        Some(large_value.as_slice())
    );
    assert_eq!(db.scan_prefix(&scope, b"bin\0").unwrap().len(), 2);

    temp.close();
    let reopened = storkv_core::Db::open(&temp.path, storkv_core::DbOptions::default()).unwrap();
    assert_eq!(
        reopened.get(&scope, key).unwrap().as_deref(),
        Some(large_value.as_slice())
    );
    assert_eq!(
        reopened.get(&scope, prefix_key).unwrap().as_deref(),
        Some(b"small".as_slice())
    );
    assert_eq!(reopened.scan_prefix(&scope, b"bin\0").unwrap().len(), 2);
}

#[test]
fn test_bloom_filter_false_positive_rate_below_two_percent() {
    let mut builder = BloomFilterBuilder::new();
    for index in 0..10_000 {
        builder.add_key(format!("present:{:05}", index).as_bytes());
    }
    let reader = BloomFilterReader::new(Bytes::from(builder.build())).unwrap();

    for index in 0..10_000 {
        assert!(
            reader.may_contain(format!("present:{:05}", index).as_bytes()),
            "inserted key must never be reported absent"
        );
    }

    let mut false_positives = 0usize;
    for index in 0..10_000 {
        if reader.may_contain(format!("absent:{:05}", index).as_bytes()) {
            false_positives += 1;
        }
    }
    let rate = false_positives as f64 / 10_000.0;
    assert!(rate < 0.02, "Bloom false-positive rate {rate} exceeds 2%");
}

#[test]
fn test_compression_single_bit_corruption_detected_at_every_offset() {
    let payload = b"storkv block payload that is long enough to compress";
    for compression in [
        CompressionType::None,
        CompressionType::Lz4,
        CompressionType::Zstd,
    ] {
        let framed = compress_and_frame_block(payload, compression).unwrap();
        for offset in 0..framed.len() {
            let mut corrupted = framed.clone();
            corrupted[offset] ^= 0x01;
            assert!(
                decompress_and_verify_block(&corrupted).is_err(),
                "{compression:?} corruption at offset {offset} must be rejected"
            );
        }
    }
}

#[test]
fn test_malformed_block_headers_are_rejected_without_panic() {
    let mut filter = vec![0u8; 8];
    filter[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(BloomFilterReader::new(Bytes::from(filter)).is_err());

    let zero_restarts = vec![0u8; 4];
    assert!(DataBlockReader::new(Bytes::from(zero_restarts)).is_err());

    let mut index = vec![0u8; 4];
    index[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(IndexBlockReader::decode(&index).is_err());
}
