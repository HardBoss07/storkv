use bytes::Bytes;
use storkv_core::wal::{WalBatchFrame, WalOp};

#[test]
fn test_compressed_wal_frame_roundtrip_and_every_byte_corruption() {
    let ops: Vec<WalOp> = (0..200)
        .map(|index| WalOp::Put {
            scope_id: 1,
            expires_at_ms: 0,
            key: Bytes::from(format!("compress_key_{index:04}")),
            val: Bytes::from(format!("compress_value_{index:04}")),
        })
        .collect();
    let frame = WalBatchFrame::new(42, ops);
    let encoded = frame.encode(true).unwrap();
    assert!(
        encoded.len() > 128,
        "payload should have qualified for compression"
    );

    let (decoded, consumed) = WalBatchFrame::decode(&encoded).unwrap();
    assert_eq!(consumed, encoded.len());
    assert_eq!(decoded.batch_seq, 42);
    assert_eq!(decoded.ops.len(), 200);

    for offset in 0..encoded.len() {
        let mut corrupted = encoded.clone();
        corrupted[offset] ^= 0x01;
        assert!(
            WalBatchFrame::decode(&corrupted).is_err(),
            "WAL frame corruption at offset {offset} must be rejected"
        );
    }
}

#[test]
fn test_wal_truncated_prefixes_never_panic() {
    let frame = WalBatchFrame::new(
        7,
        vec![WalOp::Delete {
            scope_id: 0,
            key: Bytes::from_static(b"truncate_me"),
        }],
    );
    let encoded = frame.encode(false).unwrap();
    for length in 0..encoded.len() {
        assert!(
            WalBatchFrame::decode(&encoded[..length]).is_err(),
            "prefix of length {length} must be rejected"
        );
    }
    assert!(WalBatchFrame::decode(&encoded).is_ok());
}

#[test]
fn test_wal_empty_batch_roundtrip() {
    let frame = WalBatchFrame::new(0, Vec::new());
    for compress in [false, true] {
        let encoded = frame.encode(compress).unwrap();
        let (decoded, consumed) = WalBatchFrame::decode(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(decoded.batch_seq, 0);
        assert!(decoded.ops.is_empty());
    }
}
