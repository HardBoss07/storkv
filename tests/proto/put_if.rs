use bytes::Bytes;
use std::time::Duration;
use storkv_proto::{Message, Request, Response, WirePutCondition, WirePutIfResult};

fn request_roundtrip(request: Request) {
    let encoded = Message::new_request(1, request.clone()).encode();
    let mut buffer = bytes::BytesMut::from(&encoded[..]);
    let decoded = Message::decode(&mut buffer)
        .expect("decode frame")
        .expect("one message");
    assert_eq!(decoded, Message::new_request(1, request));
}

fn response_roundtrip(response: Response) {
    let encoded = Message::new_response(1, response.clone()).encode();
    let mut buffer = bytes::BytesMut::from(&encoded[..]);
    let decoded = Message::decode(&mut buffer)
        .expect("decode frame")
        .expect("one message");
    assert_eq!(decoded, Message::new_response(1, response));
}

#[test]
fn put_if_request_round_trip() {
    let conditions = [
        WirePutCondition::Absent,
        WirePutCondition::Present,
        WirePutCondition::ExpectedSeq(42),
        WirePutCondition::ExpectedValue(Bytes::from_static(b"expected")),
    ];
    for condition in conditions {
        request_roundtrip(Request::PutIf {
            scope_id: 3,
            key: Bytes::from_static(b"key"),
            condition,
            val: Bytes::from_static(b"value"),
            ttl: Some(Duration::from_millis(500)),
        });
        request_roundtrip(Request::PutIf {
            scope_id: 3,
            key: Bytes::from_static(b"key"),
            condition: WirePutCondition::Absent,
            val: Bytes::from_static(b"value"),
            ttl: None,
        });
    }
}

#[test]
fn put_if_response_round_trip() {
    response_roundtrip(Response::PutIf(WirePutIfResult::Applied { seq: 9 }));
    response_roundtrip(Response::PutIf(WirePutIfResult::Conflict {
        current_seq: Some(4),
        current_value: Some(Bytes::from_static(b"current")),
    }));
    response_roundtrip(Response::PutIf(WirePutIfResult::Conflict {
        current_seq: None,
        current_value: None,
    }));
}
