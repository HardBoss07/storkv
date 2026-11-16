use storkv_client::{AsyncClient, ScopeHandle};

use super::support::spawn_test_server;

#[tokio::test]
async fn test_async_client_admin_stats() {
    let (server, addr, _temp_db) = spawn_test_server("client_admin_stats").await;
    let client = AsyncClient::connect(&addr).await.unwrap();

    let default_scope = ScopeHandle {
        scope_id: 0,
        name: "default".into(),
    };
    client.put(&default_scope, b"stat:k", b"v").await.unwrap();
    let _ = client.get(&default_scope, b"stat:k").await.unwrap();

    let stats = client.admin_stats().await.unwrap();
    assert_eq!(stats.levels.len(), 7);
    let default = stats
        .scopes
        .iter()
        .find(|scope| scope.scope_id == 0)
        .expect("default scope present");
    assert_eq!(default.name, "default");
    assert!(default.writes_total >= 1);
    assert!(default.reads_total >= 1);
    assert!(stats.fsync_latency_samples >= 1);

    server.shutdown_handle().send(()).unwrap();
}
