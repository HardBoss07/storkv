use bytes::{Bytes, BytesMut};
use storkv_proto::{
    Message, MessagePayload, Response, StorKvWireError, WireAdminStats, WireChangeEvent,
    WireLevelStat, WireScopeHandle, WireScopeStat, WireTelemetrySnapshot,
};

#[test]
fn test_round_trip_all_responses_and_errors() {
    let responses = vec![
        Response::Pong,
        Response::Empty,
        Response::Value(Some(Bytes::from_static(b"val"))),
        Response::Value(None),
        Response::Scan(vec![
            (Bytes::from_static(b"k1"), Bytes::from_static(b"v1")),
            (Bytes::from_static(b"k2"), Bytes::from_static(b"v2")),
        ]),
        Response::Scope(WireScopeHandle {
            scope_id: 42,
            name: "scope42".into(),
        }),
        Response::Telemetry(WireTelemetrySnapshot {
            writes_total: 10,
            reads_total: 20,
            write_conflicts_total: 1,
            compactions_total: 2,
            fsyncs_total: 5,
        }),
        Response::HealthOk,
        Response::AdminStats(WireAdminStats {
            levels: vec![
                WireLevelStat {
                    level: 0,
                    file_count: 3,
                    size_bytes: 4096,
                },
                WireLevelStat {
                    level: 6,
                    file_count: 1,
                    size_bytes: 1024,
                },
            ],
            scopes: vec![
                WireScopeStat {
                    scope_id: 0,
                    name: "default".into(),
                    writes_total: 10,
                    reads_total: 20,
                    sst_files: 3,
                },
                WireScopeStat {
                    scope_id: 7,
                    name: "sessions".into(),
                    writes_total: 0,
                    reads_total: 0,
                    sst_files: 0,
                },
            ],
            fsync_latency_p99_us: 250,
            fsync_latency_samples: 128,
        }),
        Response::WatchEvent(WireChangeEvent::Put {
            scope_id: 1,
            key: Bytes::from_static(b"watch_key"),
            value: Bytes::from_static(b"watch_val"),
            seq: 999,
        }),
        Response::WatchEvent(WireChangeEvent::Delete {
            scope_id: 1,
            key: Bytes::from_static(b"watch_key"),
            seq: 1000,
        }),
        Response::WatchEvent(WireChangeEvent::DropScope {
            scope_id: 1,
            seq: 1001,
        }),
        Response::Error(StorKvWireError::DatabaseLocked("/path/to/lock".into())),
        Response::Error(StorKvWireError::Corruption("bad CRC".into())),
        Response::Error(StorKvWireError::WriteConflict {
            scope_id: 3,
            key: b"conflict_key".to_vec(),
        }),
        Response::Error(StorKvWireError::ScopeNotFound("ghost".into())),
        Response::Error(StorKvWireError::ScopeIdNotFound(999)),
        Response::Error(StorKvWireError::ScopeAlreadyExists("default".into())),
        Response::Error(StorKvWireError::ScopeDropped(4)),
        Response::Error(StorKvWireError::SnapshotNotFound("snapX".into())),
        Response::Error(StorKvWireError::KeyTooLarge {
            max: 65535,
            actual: 70000,
        }),
        Response::Error(StorKvWireError::ValueTooLarge {
            max: 268435456,
            actual: 300000000,
        }),
        Response::Error(StorKvWireError::InvalidOperation("bad op".into())),
        Response::Error(StorKvWireError::Io("disk full".into())),
        Response::Error(StorKvWireError::Protocol("unknown header".into())),
    ];

    for (index, response) in responses.into_iter().enumerate() {
        let message = Message::new_response(index as u64 + 500, response.clone());
        let encoded = message.encode();
        let mut buffer = BytesMut::from(encoded.as_ref());

        let decoded = Message::decode(&mut buffer).expect("Failed to decode response message");
        assert!(decoded.is_some());
        let decoded = decoded.unwrap();

        assert_eq!(decoded.request_id, index as u64 + 500);
        match decoded.payload {
            MessagePayload::Response(decoded_response) => {
                assert_eq!(decoded_response, response);
            }
            _ => panic!("Expected Response payload"),
        }
        assert!(buffer.is_empty());
    }
}
