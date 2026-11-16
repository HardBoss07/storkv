use crate::common;
use bytes::Bytes;
use std::sync::Arc;
use std::time::Duration;
use storkv_client::{ScopeHandle, SyncClient};
use storkv_server::Server;
use tokio::net::TcpListener;

#[test]
fn test_sync_client_smoke() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut temp_db = common::TempDb::new("sync_client");
    let server = Arc::new(Server::with_db(Arc::new(temp_db.db.take().unwrap())));

    let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let server_clone = server.clone();
    runtime.spawn(async move {
        server_clone
            .run_tcp_listener(listener, false)
            .await
            .unwrap();
    });

    let sync_client = SyncClient::connect(&addr).expect("SyncClient connect failed");
    sync_client.ping().unwrap();
    sync_client.health_check().unwrap();

    let default_scope = ScopeHandle {
        scope_id: 0,
        name: "default".into(),
    };
    sync_client
        .put(&default_scope, b"k_sync", b"v_sync")
        .unwrap();
    let value = sync_client.get(&default_scope, b"k_sync").unwrap();
    assert_eq!(value, Some(Bytes::from_static(b"v_sync")));

    server.shutdown_handle().send(()).unwrap();
}

#[test]
fn test_sync_client_scans_range_ttl_and_delete_range() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut temp_db = common::TempDb::new("sync_client_full");
    let server = Arc::new(Server::with_db(Arc::new(temp_db.db.take().unwrap())));

    let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let server_clone = server.clone();
    runtime.spawn(async move {
        server_clone
            .run_tcp_listener(listener, false)
            .await
            .unwrap();
    });

    let client = SyncClient::connect(&addr).expect("connect");
    let scope = ScopeHandle {
        scope_id: 0,
        name: "default".into(),
    };

    for index in 0..50 {
        client
            .put(&scope, format!("sync:{:02}", index).as_bytes(), b"v")
            .unwrap();
    }
    assert_eq!(client.scan_prefix(&scope, b"sync:").unwrap().len(), 50);
    assert_eq!(
        client
            .scan(&scope, Some(b"sync:10"), Some(b"sync:20"))
            .unwrap()
            .len(),
        10
    );

    client.delete_range(&scope, b"sync:00", b"sync:10").unwrap();
    assert_eq!(client.scan_prefix(&scope, b"sync:").unwrap().len(), 40);

    client
        .put_with_ttl(&scope, b"sync:ttl", b"expiring", Duration::from_millis(50))
        .unwrap();
    assert_eq!(
        client.get(&scope, b"sync:ttl").unwrap(),
        Some(Bytes::from_static(b"expiring"))
    );
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(client.get(&scope, b"sync:ttl").unwrap(), None);

    server.shutdown_handle().send(()).unwrap();
}
