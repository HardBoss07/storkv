use bytes::{BufMut, Bytes, BytesMut};
use std::time::Duration;
use storkv_proto::{
    Message, MessagePayload, PROTOCOL_MAGIC, PROTOCOL_VERSION, Request, WireCompactionStyle,
    WireDurability, WireMergeOp, WireRetention, WireScopeOptions, WireTxMutation,
};

#[test]
fn test_round_trip_all_requests() {
    let requests = vec![
        Request::Ping,
        Request::Get {
            scope_id: 1,
            key: Bytes::from_static(b"user:100"),
        },
        Request::Put {
            scope_id: 2,
            key: Bytes::from_static(b"k1"),
            val: Bytes::from_static(b"v1"),
        },
        Request::PutWithTtl {
            scope_id: 3,
            key: Bytes::from_static(b"k2"),
            val: Bytes::from_static(b"v2"),
            ttl: Duration::from_secs(60),
        },
        Request::Delete {
            scope_id: 4,
            key: Bytes::from_static(b"k3"),
        },
        Request::DeleteRange {
            scope_id: 5,
            start_key: Bytes::from_static(b"a"),
            end_key: Bytes::from_static(b"z"),
        },
        Request::Merge {
            scope_id: 6,
            key: Bytes::from_static(b"counter"),
            op: WireMergeOp::CounterAdd(42),
        },
        Request::Merge {
            scope_id: 6,
            key: Bytes::from_static(b"custom"),
            op: WireMergeOp::Custom(Bytes::from_static(b"custom_payload")),
        },
        Request::Merge {
            scope_id: 6,
            key: Bytes::from_static(b"append_unbounded"),
            op: WireMergeOp::AppendBytes {
                data: Bytes::from_static(b"chunk"),
                max_len: None,
            },
        },
        Request::Merge {
            scope_id: 6,
            key: Bytes::from_static(b"append_bounded"),
            op: WireMergeOp::AppendBytes {
                data: Bytes::from_static(b"chunk"),
                max_len: Some(256),
            },
        },
        Request::Merge {
            scope_id: 6,
            key: Bytes::from_static(b"max_key"),
            op: WireMergeOp::MaxI64(-99),
        },
        Request::Merge {
            scope_id: 6,
            key: Bytes::from_static(b"min_key"),
            op: WireMergeOp::MinI64(99),
        },
        Request::Merge {
            scope_id: 6,
            key: Bytes::from_static(b"or_key"),
            op: WireMergeOp::BitOr(0xDEAD_BEEF),
        },
        Request::Merge {
            scope_id: 6,
            key: Bytes::from_static(b"and_key"),
            op: WireMergeOp::BitAnd(0x00FF_FF00),
        },
        Request::Scan {
            scope_id: 7,
            start_key: Some(Bytes::from_static(b"start")),
            end_key: None,
            limit: Some(10),
            reverse: true,
            keys_only: false,
        },
        Request::ScanPrefix {
            scope_id: 8,
            prefix: Bytes::from_static(b"pre:"),
            limit: None,
            reverse: false,
            keys_only: true,
        },
        Request::TxCommit {
            read_seq: 100,
            read_set: vec![(1, Bytes::from_static(b"read_k"))],
            write_set: vec![
                (
                    (1, Bytes::from_static(b"write_k")),
                    WireTxMutation::Put {
                        val: Bytes::from_static(b"write_v"),
                        expires_at_ms: 123456789,
                    },
                ),
                ((1, Bytes::from_static(b"del_k")), WireTxMutation::Delete),
                (
                    (2, Bytes::from_static(b"merge_k")),
                    WireTxMutation::Merge {
                        operand: Bytes::from_static(b"op"),
                    },
                ),
                (
                    (2, Bytes::from_static(b"range_k")),
                    WireTxMutation::DeleteRange {
                        end_key: Bytes::from_static(b"range_end"),
                    },
                ),
            ],
        },
        Request::CreateScope {
            name: "analytics".into(),
            options: WireScopeOptions {
                durability: WireDurability::Buffered {
                    sync_interval: Duration::from_millis(50),
                },
                retention: WireRetention::Ttl {
                    default_ttl: Duration::from_secs(3600),
                    compaction_style: WireCompactionStyle::Fifo,
                },
                prefix_bloom_len: Some(8),
                min_ttl: Some(Duration::from_millis(10)),
                max_ttl: Some(Duration::from_secs(86400)),
                allow_permanent: false,
                active_ttl_tracking: false,
            },
        },
        Request::DropScope {
            name: "analytics".into(),
        },
        Request::CreateSnapshot {
            name: "snap1".into(),
        },
        Request::UnpinSnapshot {
            name: "snap1".into(),
        },
        Request::Checkpoint {
            dest_dir: "/tmp/backup".into(),
        },
        Request::Compact,
        Request::CompactRange {
            scope: "analytics".into(),
            start: Some(bytes::Bytes::from_static(b"a")),
            end: Some(bytes::Bytes::from_static(b"z")),
        },
        Request::CompactRange {
            scope: "analytics".into(),
            start: None,
            end: None,
        },
        Request::WatchSubscribe,
        Request::TelemetryQuery,
        Request::HealthCheck,
        Request::AdminStats,
        Request::TriggerBackup { wait: true },
        Request::TriggerBackup { wait: false },
        Request::BackupStatus,
        Request::VerifyBackup { target_seq: None },
        Request::VerifyBackup {
            target_seq: Some(42),
        },
        Request::ListBackups,
    ];

    for (index, request) in requests.into_iter().enumerate() {
        let message = Message::new_request(index as u64 + 100, request.clone());
        let encoded = message.encode();
        let mut buffer = BytesMut::from(encoded.as_ref());

        let decoded = Message::decode(&mut buffer).expect("Failed to decode message");
        assert!(decoded.is_some(), "Expected Some(Message)");
        let decoded = decoded.unwrap();

        assert_eq!(decoded.request_id, index as u64 + 100);
        match decoded.payload {
            MessagePayload::Request(decoded_request) => {
                assert_eq!(decoded_request, request);
            }
            _ => panic!("Expected Request payload"),
        }
        assert!(buffer.is_empty(), "Buffer should be fully consumed");
    }
}

#[test]
fn test_legacy_create_scope_without_guardrails_uses_defaults() {
    let mut frame = BytesMut::new();
    frame.put_slice(PROTOCOL_MAGIC);
    frame.put_u16(PROTOCOL_VERSION);
    frame.put_u64_le(7);
    frame.put_u8(1);
    frame.put_u8(10);
    let name = b"legacy";
    frame.put_u32_le(name.len() as u32);
    frame.put_slice(name);
    frame.put_u8(1);
    frame.put_u8(1);
    frame.put_u8(0);

    let mut framed = BytesMut::new();
    framed.put_u32(frame.len() as u32);
    framed.put_slice(&frame);

    let decoded = Message::decode(&mut framed)
        .expect("legacy frame must decode")
        .expect("legacy frame must be complete");
    let MessagePayload::Request(Request::CreateScope { options, .. }) = decoded.payload else {
        panic!("expected a CreateScope request");
    };
    assert!(options.allow_permanent);
    assert!(options.min_ttl.is_none());
    assert!(options.max_ttl.is_none());
}
