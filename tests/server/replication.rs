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

#[test]
fn snapshot_removes_stale_keys_and_scopes_and_preserves_ttl() {
    let primary_temp = crate::common::TempDb::new("repl_snapshot_primary");
    let primary = primary_temp.db();
    let primary_shared = primary
        .create_scope("shared", ScopeOptions::default())
        .expect("create primary shared");
    let primary_empty = primary
        .create_scope("empty", ScopeOptions::default())
        .expect("create primary empty");
    primary
        .put(&primary_shared, b"keep", b"value")
        .expect("put keep");
    primary
        .put_with_ttl(&primary_shared, b"ttl", b"value", Duration::from_secs(3600))
        .expect("put ttl");
    primary
        .put(&primary_shared, b"extra", b"value")
        .expect("put extra");
    let (snapshot_seq, scopes, entries) = primary.replication_snapshot().expect("primary snapshot");
    assert!(
        scopes.contains(&primary_shared.scope_id),
        "snapshot must list live scopes"
    );
    assert!(
        scopes.contains(&primary_empty.scope_id),
        "snapshot must list empty scopes"
    );

    let replica_temp = crate::common::TempDb::new("repl_snapshot_replica");
    let replica = replica_temp.db();
    let replica_shared = replica
        .create_scope("shared", ScopeOptions::default())
        .expect("create replica shared");
    let replica_empty = replica
        .create_scope("empty", ScopeOptions::default())
        .expect("create replica empty");
    let stale_scope = replica
        .create_scope("stale", ScopeOptions::default())
        .expect("create stale scope");
    assert_eq!(replica_shared.scope_id, primary_shared.scope_id);
    assert_eq!(replica_empty.scope_id, primary_empty.scope_id);

    replica
        .put(&replica_shared, b"stale_key", b"stale")
        .expect("put stale key");
    replica
        .put(&stale_scope, b"stale", b"stale")
        .expect("put stale scope key");

    replica
        .apply_replication_snapshot(snapshot_seq, &scopes, &entries)
        .expect("apply snapshot");

    assert_eq!(
        replica
            .get(&replica_shared, b"keep")
            .expect("read keep")
            .as_deref(),
        Some(&b"value"[..])
    );
    assert!(
        replica
            .get(&replica_shared, b"stale_key")
            .expect("read stale key")
            .is_none(),
        "keys absent from the snapshot must be removed"
    );
    assert!(
        replica
            .get(&stale_scope, b"stale")
            .expect("read stale scope")
            .is_none(),
        "scopes absent from the snapshot must be dropped"
    );

    let now_ms = storkv_core::TimestampMs::now();
    let meta = replica
        .key_meta(&replica_shared, b"ttl")
        .expect("read ttl meta")
        .expect("ttl meta");
    assert!(
        meta.expires_at_ms > now_ms.get(),
        "snapshot must preserve the absolute expiry"
    );
    assert!(
        !replica
            .inner()
            .scopes
            .read()
            .get(&replica_empty.scope_id)
            .expect("empty scope")
            .is_dropped(),
        "empty scopes present in the snapshot must stay live"
    );
    assert_eq!(replica.newest_sequence(), snapshot_seq);
}
