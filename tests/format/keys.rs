use crate::common;
use bytes::Bytes;
use storkv_core::ScopeOptions;
use storkv_core::format::{InternalKey, MAX_KEY_LEN, ValueType, compute_crc32c};

#[test]
fn test_key_and_value_size_limits() {
    let temp = common::TempDb::new("limits");
    let db = temp.db();

    let scope = db.create_scope("large", ScopeOptions::default()).unwrap();

    let valid_key = vec![b'k'; MAX_KEY_LEN];
    let valid_value = vec![b'v'; 100];
    assert!(db.put(&scope, &valid_key, &valid_value).is_ok());

    let too_large_key = vec![b'k'; MAX_KEY_LEN + 1];
    let key_error = db.put(&scope, &too_large_key, &valid_value);
    assert!(
        matches!(key_error, Err(storkv_core::StorkvError::KeyTooLarge { .. })),
        "Expected KeyTooLarge error"
    );

    let delete_error = db.delete(&scope, &too_large_key);
    assert!(
        matches!(
            delete_error,
            Err(storkv_core::StorkvError::KeyTooLarge { .. })
        ),
        "Expected KeyTooLarge error on delete"
    );
}

#[test]
fn test_crc32c_castagnoli() {
    let data = b"STORKSST_CASTAGNOLI_CHECKSUM_VERIFICATION";
    let computed = compute_crc32c(data);
    let expected = crc32c::crc32c(data);
    assert_eq!(computed, expected);
}

#[test]
fn test_internal_key_layout_and_lexicographical_order() {
    let key_a = Bytes::from_static(b"user_alpha");
    let key_b = Bytes::from_static(b"user_beta");

    let key_a_seq100 = InternalKey::new(key_a.clone(), 100, ValueType::Value, 0);
    let key_a_seq200 = InternalKey::new(key_a.clone(), 200, ValueType::Value, 0);
    let key_a_seq100_delete = InternalKey::new(key_a.clone(), 100, ValueType::Tombstone, 0);
    let key_b_seq50 = InternalKey::new(key_b.clone(), 50, ValueType::Value, 0);

    let encoded_a100 = key_a_seq100.encode();
    let encoded_a200 = key_a_seq200.encode();
    let encoded_a100_delete = key_a_seq100_delete.encode();
    let encoded_b50 = key_b_seq50.encode();

    assert!(encoded_a100 < encoded_b50);
    assert!(encoded_a200 < encoded_b50);

    assert!(
        encoded_a200 < encoded_a100,
        "Higher seq must yield smaller byte representation (inverted_tag u64 BE)"
    );

    assert_ne!(encoded_a100, encoded_a100_delete);

    let decoded = InternalKey::decode(encoded_a200).expect("Failed to decode internal key");
    assert_eq!(decoded.user_key, key_a);
    assert_eq!(decoded.seq, 200);
    assert_eq!(decoded.value_type, ValueType::Value);
    assert_eq!(decoded.expires_at_ms, 0);
}

#[test]
fn test_internal_key_decode_rejects_truncated_and_invalid_tags() {
    for length in 0..16 {
        let bytes = Bytes::from(vec![0u8; length]);
        assert!(
            InternalKey::decode(bytes).is_none(),
            "len {length} must fail"
        );
    }

    let tag = !((5u64 << 8) | 0x7f);
    let mut raw = tag.to_be_bytes().to_vec();
    raw.extend_from_slice(&0u64.to_le_bytes());
    assert!(InternalKey::decode(Bytes::from(raw)).is_none());
}
