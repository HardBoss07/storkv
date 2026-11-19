use std::sync::Arc;
use std::time::Duration;

use storkv_client::{AsyncClient, ClientError};
use storkv_core::{Db, ScopeHandle, ScopeOptions};
use storkv_proto::{Consistency, StorkvWireError};
use storkv_server::{ReplicationRole, Server};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

struct Node {
    server: Arc<Server>,
    db: Arc<Db>,
    http: String,
    _temp: crate::common::TempDb,
    _listener: JoinHandle<()>,
}

async fn start_node(
    prefix: &str,
    role: ReplicationRole,
    primary_uri: Option<String>,
    scope_name: &str,
) -> (Node, ScopeHandle) {
    let mut temp = crate::common::TempDb::new(prefix);
    let db = Arc::new(temp.db.take().expect("test db"));
    let scope = db
        .create_scope(scope_name, ScopeOptions::default())
        .expect("create scope");
    let server = Arc::new(
        Server::with_db(db.clone())
            .with_role(role)
            .with_primary_uri(primary_uri),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address").to_string();
    let serving = server.clone();
    let handle = tokio::spawn(async move {
        let _ = serving.run_tcp_listener(listener, false).await;
    });
    let node = Node {
        server,
        db,
        http: format!("storkv://{address}"),
        _temp: temp,
        _listener: handle,
    };
    (node, scope)
}

fn client_handle(scope: &ScopeHandle) -> storkv_client::ScopeHandle {
    storkv_client::ScopeHandle {
        scope_id: scope.scope_id,
        name: scope.name.clone(),
    }
}

async fn wait_for_value(node: &Node, scope: &ScopeHandle, expected: &[u8]) -> bool {
    for _ in 0..250 {
        if node.db.get(scope, b"key").expect("replica read").as_deref() == Some(expected) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn primary_streams_writes_to_replica() {
    let (primary, scope) =
        start_node("repl_primary", ReplicationRole::Primary, None, "shared").await;
    let replica_uri = primary.http.clone();
    let (replica, _) = start_node(
        "repl_replica",
        ReplicationRole::Replica,
        Some(replica_uri),
        "shared",
    )
    .await;
    let replica_task = replica.server.spawn_replica().expect("replica task");

    let client = AsyncClient::connect_uri(&primary.http)
        .await
        .expect("connect");
    let handle = client_handle(&scope);
    client.put(&handle, b"key", b"value").await.expect("put");

    assert!(
        wait_for_value(&replica, &scope, b"value").await,
        "replica must apply the primary write"
    );

    replica_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replica_enforces_consistency_levels() {
    let (primary, scope) =
        start_node("repl_consistency", ReplicationRole::Primary, None, "shared").await;
    let replica_uri = primary.http.clone();
    let (replica, _) = start_node(
        "repl_consistency_replica",
        ReplicationRole::Replica,
        Some(replica_uri),
        "shared",
    )
    .await;
    let replica_task = replica.server.spawn_replica().expect("replica task");

    let writer = AsyncClient::connect_uri(&primary.http)
        .await
        .expect("connect");
    let handle = client_handle(&scope);
    writer.put(&handle, b"key", b"value").await.expect("put");
    assert!(wait_for_value(&replica, &scope, b"value").await);

    let client = AsyncClient::connect_uri(&replica.http)
        .await
        .expect("connect");
    match client.put(&handle, b"key", b"other").await {
        Err(ClientError::Server(StorkvWireError::NotPrimary)) => {}
        other => panic!("replica must reject writes: {other:?}"),
    }
    assert_eq!(
        client.get(&handle, b"key").await.expect("eventual read"),
        Some(bytes::Bytes::from_static(b"value"))
    );

    client
        .set_consistency(Consistency::Strong)
        .await
        .expect("configure");
    match client.get(&handle, b"key").await {
        Err(ClientError::Server(StorkvWireError::NotPrimary)) => {}
        other => panic!("strong read on a replica must fail: {other:?}"),
    }

    client
        .set_consistency(Consistency::BoundedStaleness { max_lag_ms: 0 })
        .await
        .expect("configure");
    tokio::time::sleep(Duration::from_millis(30)).await;
    match client.get(&handle, b"key").await {
        Err(ClientError::Server(StorkvWireError::StaleReplica { .. })) => {}
        other => panic!("bounded read must reject stale data: {other:?}"),
    }

    client
        .set_consistency(Consistency::Eventual)
        .await
        .expect("configure");
    assert_eq!(
        client.get(&handle, b"key").await.expect("eventual read"),
        Some(bytes::Bytes::from_static(b"value"))
    );

    replica_task.abort();
}
