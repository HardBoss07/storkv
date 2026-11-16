use bytes::Bytes;
use std::time::Duration;
use storkv_client::{AsyncClient, ScopeHandle};
use storkv_proto::WireMergeOp;

use super::support::spawn_test_server;

#[tokio::test]
async fn test_async_client_crud_and_ttl() {
    let (server, addr, _temp_db) = spawn_test_server("client_crud").await;
    let client = AsyncClient::connect(&addr).await.unwrap();

    let default_scope = ScopeHandle {
        scope_id: 0,
        name: "default".into(),
    };

    client
        .put(&default_scope, b"user:1", b"Alice")
        .await
        .unwrap();
    let value = client.get(&default_scope, b"user:1").await.unwrap();
    assert_eq!(value, Some(Bytes::from_static(b"Alice")));

    client
        .put_with_ttl(
            &default_scope,
            b"temp_sess",
            b"token123",
            Duration::from_millis(100),
        )
        .await
        .unwrap();
    let value_before = client.get(&default_scope, b"temp_sess").await.unwrap();
    assert_eq!(value_before, Some(Bytes::from_static(b"token123")));

    tokio::time::sleep(Duration::from_millis(150)).await;
    let value_after = client.get(&default_scope, b"temp_sess").await.unwrap();
    assert_eq!(value_after, None, "Expired TTL key should be invisible");

    client.delete(&default_scope, b"user:1").await.unwrap();
    let deleted_value = client.get(&default_scope, b"user:1").await.unwrap();
    assert_eq!(deleted_value, None);

    server.shutdown_handle().send(()).unwrap();
}

#[tokio::test]
async fn test_async_client_merge_and_scans() {
    let (server, addr, _temp_db) = spawn_test_server("client_merge_scan").await;
    let client = AsyncClient::connect(&addr).await.unwrap();

    let default_scope = ScopeHandle {
        scope_id: 0,
        name: "default".into(),
    };

    client
        .put(&default_scope, b"counter:page_views", b"10")
        .await
        .unwrap();
    client
        .merge(
            &default_scope,
            b"counter:page_views",
            WireMergeOp::CounterAdd(5),
        )
        .await
        .unwrap();
    let result = client
        .get(&default_scope, b"counter:page_views")
        .await
        .unwrap();
    assert_eq!(result, Some(Bytes::from_static(b"15")));

    client.put(&default_scope, b"row:001", b"v1").await.unwrap();
    client.put(&default_scope, b"row:002", b"v2").await.unwrap();
    client.put(&default_scope, b"row:003", b"v3").await.unwrap();
    client
        .put(&default_scope, b"other:01", b"v4")
        .await
        .unwrap();

    let prefix_rows = client.scan_prefix(&default_scope, b"row:").await.unwrap();
    assert_eq!(prefix_rows.len(), 3);

    let range_rows = client
        .scan(&default_scope, Some(b"row:001"), Some(b"row:003"))
        .await
        .unwrap();
    assert_eq!(range_rows.len(), 2);
    assert_eq!(range_rows[0].0, Bytes::from_static(b"row:001"));
    assert_eq!(range_rows[1].0, Bytes::from_static(b"row:002"));

    client
        .delete_range(&default_scope, b"row:001", b"row:003")
        .await
        .unwrap();
    let after_delete_range = client.scan_prefix(&default_scope, b"row:").await.unwrap();
    assert_eq!(after_delete_range.len(), 1);
    assert_eq!(after_delete_range[0].0, Bytes::from_static(b"row:003"));

    server.shutdown_handle().send(()).unwrap();
}
