use bytes::{Bytes, BytesMut};
use storkv_proto::{
    Message, MessagePayload, Response, StorkvWireError, WireAdminStats, WireBackupEntry,
    WireBackupKind, WireBackupReport, WireBackupStatus, WireBackupVerifyError,
    WireBackupVerifyReport, WireChangeEvent, WireLevelStat, WireScopeHandle, WireScopeStat,
    WireTelemetrySnapshot, WireTransportStat,
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
            backups_completed_total: 3,
            backups_failed_total: 1,
            last_backup_timestamp_ms: 1_760_000_000_000,
            last_backup_seq: 42,
            wal_segments_shipped_total: 7,
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
            transports: vec![WireTransportStat {
                kind: 1,
                connections_total: 9,
                active_connections: 2,
                requests_total: 500,
            }],
            backups_completed_total: 3,
            backups_failed_total: 1,
            last_backup_timestamp_ms: 1_760_000_000_000,
            last_backup_seq: 42,
            wal_segments_shipped_total: 7,
        }),
        Response::BackupStatus(WireBackupStatus {
            enabled: true,
            backup_root: Some("/srv/backups".into()),
            last_backup_timestamp_ms: 1_760_000_000_000,
            last_backup_seq: 42,
            backups_completed_total: 3,
            backups_failed_total: 1,
            wal_segments_shipped_total: 7,
            next_full_backup_in_ms: 3_600_000,
            last_error: Some("transient".into()),
            sink: "local".into(),
        }),
        Response::BackupReport(WireBackupReport {
            generation_id: "full-1760000000000-0000000000000042".into(),
            sequence: 42,
            files: 4,
            bytes: 1_048_576,
            generations_removed: 1,
            wal_segments_removed: 2,
            bytes_freed: 4096,
        }),
        Response::BackupVerifyReport(WireBackupVerifyReport {
            generations_checked: 2,
            files_checked: 8,
            bytes_checked: 2_097_152,
            wal_segments_checked: 5,
            errors: vec![WireBackupVerifyError {
                path: "scopes/0/1.sst".into(),
                detail: "crc mismatch".into(),
            }],
        }),
        Response::BackupCatalog(vec![WireBackupEntry {
            generation_id: "full-1760000000000-0000000000000042".into(),
            kind: WireBackupKind::Full,
            created_at_ms: 1_760_000_000_000,
            min_seq: 1,
            max_seq: 42,
            total_bytes: 1_048_576,
            files: 4,
            compression: "zstd".into(),
        }]),
        Response::WatchEvent(WireChangeEvent::Put {
            scope_id: 1,
            key: Bytes::from_static(b"watch_key"),
            value: Bytes::from_static(b"watch_val"),
            seq: 999,
            expires_at_ms: 0,
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
        Response::ReplicationSnapshot {
            snapshot_seq: 1002,
            scopes: vec![1, 2],
            entries: vec![
                (
                    1,
                    Bytes::from_static(b"key"),
                    Bytes::from_static(b"value"),
                    1_700_000_000_000,
                ),
                (2, Bytes::from_static(b"other"), Bytes::from_static(b"v"), 0),
            ],
        },
        Response::Error(StorkvWireError::DatabaseLocked("/path/to/lock".into())),
        Response::Error(StorkvWireError::Corruption("bad CRC".into())),
        Response::Error(StorkvWireError::WriteConflict {
            scope_id: 3,
            key: b"conflict_key".to_vec(),
        }),
        Response::Error(StorkvWireError::ScopeNotFound("ghost".into())),
        Response::Error(StorkvWireError::ScopeIdNotFound(999)),
        Response::Error(StorkvWireError::ScopeAlreadyExists("default".into())),
        Response::Error(StorkvWireError::ScopeDropped(4)),
        Response::Error(StorkvWireError::SnapshotNotFound("snapX".into())),
        Response::Error(StorkvWireError::KeyTooLarge {
            max: 65535,
            actual: 70000,
        }),
        Response::Error(StorkvWireError::ValueTooLarge {
            max: 268435456,
            actual: 300000000,
        }),
        Response::Error(StorkvWireError::InvalidOperation("bad op".into())),
        Response::Error(StorkvWireError::Io("disk full".into())),
        Response::Error(StorkvWireError::Protocol("unknown header".into())),
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
