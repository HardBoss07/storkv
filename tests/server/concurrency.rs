use crate::common;
use bytes::Bytes;
use std::sync::Arc;
use storkv_proto::{Request, Response};
use storkv_server::Server;
use tokio::net::{TcpListener, TcpStream};

use super::support::{send_request, spawn_server};

#[tokio::test]
async fn test_multi_client_concurrent_requests() {
    let mut temp_db = common::TempDb::new("server_concurrent");
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

    let mut tasks = Vec::new();
    for index in 0..10 {
        tasks.push(tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            let key = format!("k_{}", index).into_bytes();
            let value = format!("v_{}", index).into_bytes();

            let put_response = send_request(
                &mut stream,
                index as u64,
                Request::Put {
                    scope_id: 0,
                    key: Bytes::from(key.clone()),
                    val: Bytes::from(value.clone()),
                },
            )
            .await;
            assert_eq!(put_response, Response::Empty);

            let get_response = send_request(
                &mut stream,
                index as u64 + 100,
                Request::Get {
                    scope_id: 0,
                    key: Bytes::from(key),
                },
            )
            .await;
            assert_eq!(get_response, Response::Value(Some(Bytes::from(value))));
        }));
    }

    for task in tasks {
        task.await.unwrap();
    }

    server.shutdown_handle().send(()).unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn test_32_concurrent_clients_crud_and_scan() {
    let (addr, server, _temp, handle) = spawn_server("server_32_clients").await;

    let mut tasks = Vec::new();
    for client in 0..32u64 {
        let addr = addr.clone();
        tasks.push(tokio::spawn(async move {
            let mut stream = TcpStream::connect(&addr).await.unwrap();
            let key = format!("bulk:{:02}", client);
            let value = format!("value:{:02}", client);

            assert_eq!(
                send_request(
                    &mut stream,
                    client * 10 + 1,
                    Request::Put {
                        scope_id: 0,
                        key: Bytes::from(key.clone()),
                        val: Bytes::from(value.clone()),
                    },
                )
                .await,
                Response::Empty
            );
            assert_eq!(
                send_request(
                    &mut stream,
                    client * 10 + 2,
                    Request::Get {
                        scope_id: 0,
                        key: Bytes::from(key),
                    },
                )
                .await,
                Response::Value(Some(Bytes::from(value)))
            );
            let scan = send_request(
                &mut stream,
                client * 10 + 3,
                Request::ScanPrefix {
                    scope_id: 0,
                    prefix: Bytes::from_static(b"bulk:"),
                },
            )
            .await;
            match scan {
                Response::Scan(pairs) => {
                    assert!(pairs.iter().any(|(key, _)| key.starts_with(b"bulk:")))
                }
                other => panic!("unexpected scan response: {other:?}"),
            }
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }

    server.shutdown_handle().send(()).unwrap();
    handle.await.unwrap();
}
