use bytes::Bytes;
use storkv_client::{AsyncClient, ScopeHandle};

use super::support::spawn_test_server;

#[tokio::test]
async fn test_32_concurrent_async_clients() {
    let (server, addr, _temp_db) = spawn_test_server("client_32").await;
    let default_scope = ScopeHandle {
        scope_id: 0,
        name: "default".into(),
    };

    let mut tasks = Vec::new();
    for client_id in 0..32u64 {
        let addr = addr.clone();
        let scope = default_scope.clone();
        tasks.push(tokio::spawn(async move {
            let client = AsyncClient::connect(&addr).await.unwrap();
            let key = format!("parallel:{:02}", client_id);
            let value = format!("value:{:02}", client_id);
            client
                .put(&scope, key.as_bytes(), value.as_bytes())
                .await
                .unwrap();
            assert_eq!(
                client.get(&scope, key.as_bytes()).await.unwrap(),
                Some(Bytes::from(value))
            );
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }

    let verify = AsyncClient::connect(&addr).await.unwrap();
    assert_eq!(
        verify
            .scan_prefix(&default_scope, b"parallel:")
            .await
            .unwrap()
            .len(),
        32
    );

    server.shutdown_handle().send(()).unwrap();
}
