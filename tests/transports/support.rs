use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use storkv_client::{AsyncClient, ScopeHandle};
use storkv_core::backup::BackupConfig;
use storkv_proto::{WireDurability, WireRetention};
use storkv_server::{Server, TransportOptions};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// A live server exposing native TCP, HTTP, WebSocket, and gRPC transports on
/// ephemeral loopback ports.
pub struct TestTransports {
    pub server: Arc<Server>,
    pub _temp: crate::common::TempDb,
    pub tcp_uri: String,
    pub http_uri: String,
    pub ws_uri: String,
    pub grpc_uri: String,
    pub backup_root: Option<PathBuf>,
    handles: Vec<JoinHandle<()>>,
}

impl Drop for TestTransports {
    fn drop(&mut self) {
        let _ = self.server.shutdown_handle().send(());
        for handle in &self.handles {
            handle.abort();
        }
    }
}

pub async fn spawn_all(prefix: &str) -> TestTransports {
    spawn_inner(prefix, false).await
}

/// Like [`spawn_all`], but also starts the autonomous backup worker writing
/// under a `backups` directory inside the temporary database root.
pub async fn spawn_all_with_backup(prefix: &str) -> TestTransports {
    spawn_inner(prefix, true).await
}

async fn spawn_inner(prefix: &str, with_backup: bool) -> TestTransports {
    let mut temp = crate::common::TempDb::new(prefix);
    let backup_root = with_backup.then(|| temp.path.join("backups"));
    let server = Arc::new(Server::with_db(Arc::new(temp.db.take().expect("test db"))));
    if let Some(root) = &backup_root {
        server
            .db()
            .start_auto_backup(BackupConfig::local(root.clone()))
            .expect("start auto backup");
    }
    let mut handles = Vec::new();

    let tcp_listener = TcpListener::bind("127.0.0.1:0").await.expect("tcp bind");
    let tcp_addr = tcp_listener.local_addr().expect("tcp addr").to_string();
    let tcp_server = server.clone();
    handles.push(tokio::spawn(async move {
        let _ = tcp_server.run_tcp_listener(tcp_listener, true).await;
    }));

    let http_listener = TcpListener::bind("127.0.0.1:0").await.expect("http bind");
    let http_addr = http_listener.local_addr().expect("http addr").to_string();
    let http_server = server.clone();
    handles.push(tokio::spawn(async move {
        let _ = storkv_server::transports::serve_http_tcp(
            http_server,
            http_listener,
            TransportOptions::new(true, None),
        )
        .await;
    }));

    let ws_listener = TcpListener::bind("127.0.0.1:0").await.expect("ws bind");
    let ws_addr = ws_listener.local_addr().expect("ws addr").to_string();
    let ws_server = server.clone();
    handles.push(tokio::spawn(async move {
        let _ = storkv_server::transports::serve_ws_tcp(
            ws_server,
            ws_listener,
            TransportOptions::new(true, None),
        )
        .await;
    }));

    let grpc_listener = TcpListener::bind("127.0.0.1:0").await.expect("grpc bind");
    let grpc_addr = grpc_listener.local_addr().expect("grpc addr").to_string();
    let grpc_server = server.clone();
    handles.push(tokio::spawn(async move {
        storkv_server::transports::serve_grpc_tcp(
            grpc_server,
            grpc_listener,
            TransportOptions::new(true, None),
        )
        .await
        .expect("grpc serve");
    }));

    TestTransports {
        server,
        _temp: temp,
        tcp_uri: format!("storkv://{tcp_addr}"),
        http_uri: format!("http://{http_addr}"),
        ws_uri: format!("ws://{ws_addr}/v1/ws"),
        grpc_uri: format!("grpc://{grpc_addr}"),
        backup_root,
        handles,
    }
}

pub async fn connect(uri: &str) -> AsyncClient {
    AsyncClient::connect_uri(uri)
        .await
        .unwrap_or_else(|error| panic!("connect {uri}: {error}"))
}

pub async fn make_scope(client: &AsyncClient, name: &str) -> ScopeHandle {
    client
        .create_scope(
            name,
            WireDurability::StrictFsync,
            WireRetention::Permanent,
            Some(8),
        )
        .await
        .expect("create scope")
}

pub async fn crud_roundtrip(uri: &str) {
    let client = connect(uri).await;
    let scope = make_scope(&client, &format!("scope_{}", scope_suffix(uri))).await;

    client.ping().await.expect("ping");
    client.health_check().await.expect("health");

    client.put(&scope, b"user:1", b"alice").await.expect("put");
    let value = client.get(&scope, b"user:1").await.expect("get");
    assert_eq!(value, Some(Bytes::from_static(b"alice")));

    for index in 0..5u8 {
        client
            .put(
                &scope,
                format!("batch:{index}").as_bytes(),
                format!("v{index}").as_bytes(),
            )
            .await
            .expect("put batch");
    }
    let prefix = client
        .scan_prefix(&scope, b"batch:")
        .await
        .expect("scan prefix");
    assert_eq!(prefix.len(), 5);

    let mut tx = client.begin_tx().await.expect("begin tx");
    tx.put(&scope, b"tx:key", b"tx:value");
    tx.commit().await.expect("commit tx");
    assert_eq!(
        client.get(&scope, b"tx:key").await.expect("tx get"),
        Some(Bytes::from_static(b"tx:value"))
    );

    client.delete(&scope, b"user:1").await.expect("delete user");
    assert_eq!(
        client.get(&scope, b"user:1").await.expect("get deleted"),
        None
    );

    client
        .delete_range(&scope, b"batch:0", b"batch:5")
        .await
        .expect("delete range");
    assert!(
        client
            .scan_prefix(&scope, b"batch:")
            .await
            .expect("scan after delete range")
            .is_empty()
    );

    let telemetry = client.telemetry().await.expect("telemetry");
    assert!(telemetry.writes_total > 0);
}

fn scope_suffix(uri: &str) -> String {
    uri.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect()
}

pub async fn assert_error_propagation(uri: &str) {
    let client = connect(uri).await;
    let missing = ScopeHandle {
        scope_id: 4095,
        name: "missing".into(),
    };
    assert!(client.get(&missing, b"key").await.is_err());
}
