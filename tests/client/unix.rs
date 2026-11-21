#[cfg(unix)]
use crate::common;
#[cfg(unix)]
use bytes::Bytes;
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use storkv_client::{AsyncClient, ScopeHandle};
#[cfg(unix)]
use storkv_server::Server;

#[cfg(unix)]
#[tokio::test]
async fn test_async_client_connects_over_unix_socket() {
    let mut temp_db = common::TempDb::new("client_uds");
    let server = Arc::new(Server::with_db(Arc::new(temp_db.db.take().unwrap())));

    let sock_path = temp_db.path.join("client.sock");
    let _ = std::fs::remove_file(&sock_path);
    let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

    let server_clone = server.clone();
    let handle = tokio::spawn(async move {
        server_clone
            .run_uds_listener(listener, false)
            .await
            .unwrap();
    });

    let client = AsyncClient::connect_any(&format!("unix:{}", sock_path.display()))
        .await
        .unwrap();
    client.ping().await.unwrap();

    let scope = ScopeHandle {
        scope_id: 0,
        name: "default".into(),
    };
    client.put(&scope, b"uds_key", b"uds_val").await.unwrap();
    assert_eq!(
        client.get(&scope, b"uds_key").await.unwrap(),
        Some(Bytes::from_static(b"uds_val"))
    );

    server.shutdown_handle().send(()).unwrap();
    handle.await.unwrap();
    let _ = std::fs::remove_file(&sock_path);
}
