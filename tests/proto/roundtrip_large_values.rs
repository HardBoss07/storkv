use bytes::{Bytes, BytesMut};
use storkv_proto::{Message, MessagePayload, Request};

#[test]
fn test_large_value_and_empty_key_roundtrip() {
    let large = Bytes::from(vec![0xA5u8; 1024 * 1024]);
    let request = Request::Put {
        scope_id: 9,
        key: Bytes::new(),
        val: large.clone(),
    };
    let encoded = Message::new_request(77, request.clone()).encode();
    assert!(encoded.len() > large.len());
    let mut buffer = BytesMut::from(encoded.as_ref());
    let decoded = Message::decode(&mut buffer).unwrap().unwrap();
    assert_eq!(decoded.payload, MessagePayload::Request(request));
    assert!(buffer.is_empty());

    let empty = Request::Put {
        scope_id: 0,
        key: Bytes::new(),
        val: Bytes::new(),
    };
    let encoded = Message::new_request(78, empty.clone()).encode();
    let mut buffer = BytesMut::from(encoded.as_ref());
    assert_eq!(
        Message::decode(&mut buffer).unwrap().unwrap().payload,
        MessagePayload::Request(empty)
    );
}
