use crate::common;
use bytes::Bytes;
use std::sync::Arc;
use storkv_proto::{
    Request, Response, StorkvWireError, WireDurability, WireRetention, WireScopeOptions,
    WireTxMutation,
};
use storkv_server::Server;
use tokio::net::{TcpListener, TcpStream};

use super::support::send_request;

#[tokio::test]
async fn test_cross_scope_transactions_and_occ_conflict() {
    let mut temp_db = common::TempDb::new("server_tx");
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

    let mut stream1 = TcpStream::connect(addr).await.unwrap();

    let create_scope_response = send_request(
        &mut stream1,
        1,
        Request::CreateScope {
            name: "accounts".into(),
            options: WireScopeOptions {
                durability: WireDurability::StrictFsync,
                retention: WireRetention::Permanent,
                prefix_bloom_len: None,
                min_ttl: None,
                max_ttl: None,
                allow_permanent: true,
            },
        },
    )
    .await;

    let scope_id = match create_scope_response {
        Response::Scope(handle) => handle.scope_id,
        other => panic!("Unexpected scope response: {:?}", other),
    };

    let put_response = send_request(
        &mut stream1,
        2,
        Request::Put {
            scope_id,
            key: Bytes::from_static(b"user:alice"),
            val: Bytes::from_static(b"100"),
        },
    )
    .await;
    assert_eq!(put_response, Response::Empty);

    let tx_begin_response = send_request(&mut stream1, 3, Request::TxBegin).await;
    let read_seq = match tx_begin_response {
        Response::TxStarted { read_seq } => read_seq,
        other => panic!("Expected TxStarted, got {:?}", other),
    };

    let mut stream2 = TcpStream::connect(addr).await.unwrap();
    let interim_put = send_request(
        &mut stream2,
        4,
        Request::Put {
            scope_id,
            key: Bytes::from_static(b"user:alice"),
            val: Bytes::from_static(b"150"),
        },
    )
    .await;
    assert_eq!(interim_put, Response::Empty);

    let tx_response = send_request(
        &mut stream1,
        5,
        Request::TxCommit {
            read_seq,
            read_set: vec![(scope_id, Bytes::from_static(b"user:alice"))],
            write_set: vec![(
                (scope_id, Bytes::from_static(b"user:alice")),
                WireTxMutation::Put {
                    val: Bytes::from_static(b"200"),
                    expires_at_ms: 0,
                },
            )],
        },
    )
    .await;

    match tx_response {
        Response::Error(StorkvWireError::WriteConflict { .. }) => {}
        other => panic!("Expected WriteConflict error, got {:?}", other),
    }

    server.shutdown_handle().send(()).unwrap();
    handle.await.unwrap();
}
