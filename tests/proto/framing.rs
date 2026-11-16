use bytes::{Bytes, BytesMut};
use storkv_proto::{
    Message, MessagePayload, Request, Response, WireAdminStats, WireLevelStat, WireScopeStat,
    WireTxMutation,
};

#[test]
fn test_length_prefix_boundaries_and_partial_frames() {
    let message = Message::new_request(
        1,
        Request::Put {
            scope_id: 1,
            key: Bytes::from_static(b"boundary_key"),
            val: Bytes::from_static(b"boundary_val"),
        },
    );
    let encoded = message.encode();

    let mut buffer = BytesMut::new();
    for byte in &encoded[..encoded.len() - 1] {
        buffer.extend_from_slice(&[*byte]);
        let result = Message::decode(&mut buffer).expect("Incremental feeding should not fail");
        assert!(result.is_none(), "Should return None while incomplete");
    }

    buffer.extend_from_slice(&[encoded[encoded.len() - 1]]);
    let result = Message::decode(&mut buffer).expect("Complete frame should decode cleanly");
    assert!(result.is_some(), "Should decode once all bytes are present");
    assert!(buffer.is_empty());
}

#[test]
fn test_multiple_frames_in_stream() {
    let message1 = Message::new_request(1, Request::Ping);
    let message2 = Message::new_response(1, Response::Pong);
    let message3 = Message::new_request(
        2,
        Request::Get {
            scope_id: 1,
            key: Bytes::from_static(b"foo"),
        },
    );

    let mut stream = BytesMut::new();
    stream.extend_from_slice(&message1.encode());
    stream.extend_from_slice(&message2.encode());
    stream.extend_from_slice(&message3.encode());

    let decoded1 = Message::decode(&mut stream)
        .unwrap()
        .expect("First message");
    assert_eq!(decoded1.request_id, 1);
    assert_eq!(decoded1.payload, MessagePayload::Request(Request::Ping));

    let decoded2 = Message::decode(&mut stream)
        .unwrap()
        .expect("Second message");
    assert_eq!(decoded2.request_id, 1);
    assert_eq!(decoded2.payload, MessagePayload::Response(Response::Pong));

    let decoded3 = Message::decode(&mut stream)
        .unwrap()
        .expect("Third message");
    assert_eq!(decoded3.request_id, 2);
    assert_eq!(
        decoded3.payload,
        MessagePayload::Request(Request::Get {
            scope_id: 1,
            key: Bytes::from_static(b"foo")
        })
    );

    assert!(stream.is_empty());
}

#[test]
fn test_every_prefix_of_every_representative_frame() {
    let messages = vec![
        Message::new_request(
            1,
            Request::Put {
                scope_id: 1,
                key: Bytes::from_static(b"prefix_key"),
                val: Bytes::from_static(b"prefix_val"),
            },
        ),
        Message::new_request(
            2,
            Request::TxCommit {
                read_seq: 5,
                read_set: vec![(1, Bytes::from_static(b"r"))],
                write_set: vec![(
                    (2, Bytes::from_static(b"w")),
                    WireTxMutation::Put {
                        val: Bytes::from_static(b"v"),
                        expires_at_ms: 3,
                    },
                )],
            },
        ),
        Message::new_response(
            3,
            Response::Scan(vec![
                (Bytes::from_static(b"k1"), Bytes::from_static(b"v1")),
                (Bytes::from_static(b"k2"), Bytes::from_static(b"v2")),
            ]),
        ),
        Message::new_response(
            4,
            Response::AdminStats(WireAdminStats {
                levels: vec![WireLevelStat {
                    level: 6,
                    file_count: 1,
                    size_bytes: 10,
                }],
                scopes: vec![WireScopeStat {
                    scope_id: 1,
                    name: "s".into(),
                    writes_total: 1,
                    reads_total: 2,
                    sst_files: 3,
                }],
                fsync_latency_p99_us: 9,
                fsync_latency_samples: 1,
            }),
        ),
        Message::new_response(
            5,
            Response::Error(storkv_proto::StorKvWireError::InvalidOperation("x".into())),
        ),
    ];

    for message in messages {
        let encoded = message.encode();
        for length in 0..encoded.len() {
            let mut buffer = BytesMut::from(&encoded[..length]);
            match Message::decode(&mut buffer) {
                Ok(None) => {}
                Ok(Some(_)) => panic!("prefix {length} of {} decoded early", encoded.len()),
                Err(_) => {}
            }
        }
        let mut full = BytesMut::from(encoded.as_ref());
        assert!(Message::decode(&mut full).unwrap().is_some());
        assert!(full.is_empty());
    }
}
