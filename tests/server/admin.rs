use crate::common;
use bytes::Bytes;
use std::sync::Arc;
use storkv_proto::{
    Request, Response, WireAdminStats, WireDurability, WireRetention, WireScopeOptions,
};
use storkv_server::Server;
use tokio::net::{TcpListener, TcpStream};

use super::support::send_request;

#[tokio::test]
async fn test_admin_stats_reports_levels_scopes_and_latency() {
    let mut temp_db = common::TempDb::new("server_admin_stats");
    let server = Arc::new(Server::with_db(Arc::new(temp_db.db.take().unwrap())));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_clone = server.clone();
    let handle = tokio::spawn(async move {
        server_clone
            .run_tcp_listener(listener, false)
            .await
            .unwrap();
    });

    let mut stream = TcpStream::connect(addr).await.unwrap();

    let create_response = send_request(
        &mut stream,
        1,
        Request::CreateScope {
            name: "metrics".into(),
            options: WireScopeOptions {
                durability: WireDurability::StrictFsync,
                retention: WireRetention::Permanent,
                prefix_bloom_len: None,
            },
        },
    )
    .await;
    let scope_id = match create_response {
        Response::Scope(scope) => scope.scope_id,
        other => panic!("Expected scope handle, got {:?}", other),
    };

    assert_eq!(
        send_request(
            &mut stream,
            2,
            Request::Put {
                scope_id,
                key: Bytes::from_static(b"k"),
                val: Bytes::from_static(b"v"),
            },
        )
        .await,
        Response::Empty
    );
    assert_eq!(
        send_request(
            &mut stream,
            3,
            Request::Get {
                scope_id,
                key: Bytes::from_static(b"k"),
            },
        )
        .await,
        Response::Value(Some(Bytes::from_static(b"v")))
    );

    match send_request(&mut stream, 4, Request::AdminStats).await {
        Response::AdminStats(WireAdminStats {
            levels,
            scopes,
            fsync_latency_p99_us,
            fsync_latency_samples,
        }) => {
            assert_eq!(levels.len(), 7);
            assert!(levels.iter().any(|level| level.level == 0));
            let metrics = scopes
                .iter()
                .find(|scope| scope.scope_id == scope_id)
                .expect("new scope present in admin stats");
            assert_eq!(metrics.name, "metrics");
            assert!(metrics.writes_total >= 1);
            assert!(metrics.reads_total >= 1);
            assert!(fsync_latency_samples >= 1);
            let _ = fsync_latency_p99_us;
        }
        other => panic!("Expected AdminStats, got {:?}", other),
    }

    server.shutdown_handle().send(()).unwrap();
    handle.await.unwrap();
}
