use bytes::BytesMut;
use storkv_proto::{MAX_FRAME_SIZE, Message, StorKvWireError};

use super::support::{frame_from_body, value_with_tag};

#[test]
fn test_rejection_of_oversized_or_corrupted_frame() {
    let mut buffer = BytesMut::new();
    let oversized_length = (MAX_FRAME_SIZE + 1) as u32;
    buffer.extend_from_slice(&oversized_length.to_be_bytes());
    buffer.extend_from_slice(b"dummy");
    let error = Message::decode(&mut buffer).unwrap_err();
    assert!(matches!(error, StorKvWireError::Protocol(_)));

    let mut bad_magic = BytesMut::new();
    let length = 20u32;
    bad_magic.extend_from_slice(&length.to_be_bytes());
    bad_magic.extend_from_slice(b"BADMAGIC");
    bad_magic.extend_from_slice(&1u16.to_be_bytes());
    bad_magic.extend_from_slice(&0u64.to_le_bytes());
    bad_magic.extend_from_slice(&[1u8, 0u8]);
    let error = Message::decode(&mut bad_magic).unwrap_err();
    assert!(matches!(error, StorKvWireError::Protocol(_)));
}

#[test]
fn test_malformed_type_and_opcode_frames_are_rejected() {
    let mut buffer = frame_from_body(0xEE, &[]);
    assert!(Message::decode(&mut buffer).is_err());

    let mut buffer = frame_from_body(1, &[0xFF]);
    assert!(Message::decode(&mut buffer).is_err());

    let mut buffer = frame_from_body(2, &[0xFE]);
    assert!(Message::decode(&mut buffer).is_err());

    let mut payload = Vec::new();
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.push(0x7F);
    let mut buffer = frame_from_body(1, &value_with_tag(7, &payload));
    assert!(Message::decode(&mut buffer).is_err());

    let mut payload = Vec::new();
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.push(b'k');
    payload.push(0x55);
    let mut buffer = frame_from_body(1, &value_with_tag(6, &payload));
    assert!(Message::decode(&mut buffer).is_err());

    let mut payload = Vec::new();
    payload.extend_from_slice(&1u64.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.push(b'k');
    payload.push(0x55);
    let mut buffer = frame_from_body(1, &value_with_tag(9, &payload));
    assert!(Message::decode(&mut buffer).is_err());

    let mut payload = Vec::new();
    payload.extend_from_slice(&3u32.to_le_bytes());
    payload.extend_from_slice(&[0xFF, 0xFE, 0xFD]);
    let mut buffer = frame_from_body(1, &value_with_tag(11, &payload));
    assert!(Message::decode(&mut buffer).is_err());

    let mut body = BytesMut::new();
    body.extend_from_slice(storkv_proto::PROTOCOL_MAGIC);
    body.extend_from_slice(&999u16.to_be_bytes());
    body.extend_from_slice(&1u64.to_le_bytes());
    body.extend_from_slice(&[1, 0]);
    let mut buffer = BytesMut::new();
    buffer.extend_from_slice(&(body.len() as u32).to_be_bytes());
    buffer.extend_from_slice(&body);
    assert!(Message::decode(&mut buffer).is_err());
}

#[test]
fn test_untrusted_collection_counts_are_rejected_before_allocation() {
    let mut payload = Vec::new();
    payload.extend_from_slice(&u32::MAX.to_le_bytes());
    let mut buffer = frame_from_body(2, &value_with_tag(3, &payload));
    assert!(Message::decode(&mut buffer).is_err());

    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&u32::MAX.to_le_bytes());
    let mut buffer = frame_from_body(1, &value_with_tag(9, &payload));
    assert!(Message::decode(&mut buffer).is_err());

    let mut payload = Vec::new();
    payload.extend_from_slice(&u32::MAX.to_le_bytes());
    let mut buffer = frame_from_body(2, &value_with_tag(9, &payload));
    assert!(Message::decode(&mut buffer).is_err());

    let mut payload = Vec::new();
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&u32::MAX.to_le_bytes());
    let mut buffer = frame_from_body(2, &value_with_tag(9, &payload));
    assert!(Message::decode(&mut buffer).is_err());
}
