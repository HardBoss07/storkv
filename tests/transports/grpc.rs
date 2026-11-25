use std::time::Duration;

use bytes::Bytes;
use storkv_proto::{WireBackupKind, WireChangeEvent};

use super::support;

#[tokio::test]
async fn grpc_crud_scan_and_errors() {
    let transports = support::spawn_all("transports_grpc").await;
    support::crud_roundtrip(&transports.grpc_uri).await;
    support::assert_error_propagation(&transports.grpc_uri).await;
}

#[tokio::test]
async fn grpc_admin_compact_forwards_range_bounds() {
    let transports = support::spawn_all("transports_grpc_compact_range").await;
    let client = support::connect(&transports.grpc_uri).await;
    let scope = support::make_scope(&client, "grpc_range").await;
    client
        .compact_range(&scope.name, Some(b"a"), Some(b"z"))
        .await
        .expect("valid range");
    let error = client
        .compact_range(&scope.name, Some(b"z"), Some(b"a"))
        .await
        .expect_err("inverted range must fail");
    assert!(error.to_string().contains("start must be less than end"));
}

#[tokio::test]
async fn grpc_merge_counter() {
    let transports = support::spawn_all("transports_grpc_merge").await;
    let client = support::connect(&transports.grpc_uri).await;
    let scope = support::make_scope(&client, "counters").await;

    client
        .merge(&scope, b"hits", storkv_proto::WireMergeOp::CounterAdd(7))
        .await
        .expect("merge");
    assert_eq!(
        client.get(&scope, b"hits").await.expect("get"),
        Some(Bytes::from_static(b"7"))
    );
}

#[tokio::test]
async fn grpc_backup_catalog_matches_triggered_generation() {
    let transports = support::spawn_all_with_backup("transports_grpc_backup").await;
    let backup_root = transports
        .backup_root
        .clone()
        .expect("backup-enabled transport");
    let client = support::connect(&transports.grpc_uri).await;
    let scope = support::make_scope(&client, "grpc_backup_scope").await;
    client
        .put(&scope, b"archived", b"value")
        .await
        .expect("put archived");

    let report = client.trigger_backup(true).await.expect("trigger backup");
    assert!(!report.generation_id.is_empty());
    assert!(report.sequence >= 1);

    let status = client.backup_status().await.expect("backup status");
    assert!(status.enabled);
    assert_eq!(status.backup_root.as_deref(), backup_root.to_str());
    assert!(status.backups_completed_total >= 1);

    let catalog = client.list_backups().await.expect("list backups");
    let entry = catalog
        .iter()
        .find(|entry| entry.generation_id == report.generation_id)
        .expect("triggered generation in catalog");
    assert_eq!(entry.kind, WireBackupKind::Full);
    assert_eq!(entry.generation_id, report.generation_id);
    assert!(entry.files >= 1);
    assert!(entry.max_seq >= report.sequence);

    let verification = client.verify_backup(None).await.expect("verify backups");
    assert!(verification.is_ok(), "errors: {:?}", verification.errors);
    assert!(verification.generations_checked >= 1);
}

#[tokio::test]
async fn grpc_watch_stream_receives_put_events() {
    let transports = support::spawn_all("transports_grpc_watch").await;
    let client = support::connect(&transports.grpc_uri).await;
    let scope = support::make_scope(&client, "grpc_watch_scope").await;

    let mut watch = client
        .subscribe_watch()
        .await
        .expect("subscribe grpc watch");
    // The watch stream borrows the client connection while waiting, so writes
    // go through an independent client.
    let writer = support::connect(&transports.grpc_uri).await;
    let writer_scope = scope.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        writer
            .put(&writer_scope, b"watched", b"value")
            .await
            .expect("watch put");
    });

    let event = tokio::time::timeout(Duration::from_secs(5), watch.next())
        .await
        .expect("watch timeout")
        .expect("watch transport")
        .expect("watch event");
    match event {
        WireChangeEvent::Put { key, value, .. } => {
            assert_eq!(key.as_ref(), b"watched");
            assert_eq!(value.as_ref(), b"value");
        }
        other => panic!("expected put event, got {other:?}"),
    }
}
