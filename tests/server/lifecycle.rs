use crate::common;
use std::sync::Arc;
use storkv_proto::{Request, Response};
use storkv_server::Server;
use tokio::net::{TcpListener, TcpStream};

use super::support::send_request;

#[tokio::test]
async fn test_server_startup_shutdown_and_ping() {
    let mut temp_db = common::TempDb::new("server_ping");
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
    let response = send_request(&mut stream, 1, Request::Ping).await;
    assert_eq!(response, Response::Pong);

    let health = send_request(&mut stream, 2, Request::HealthCheck).await;
    assert_eq!(health, Response::HealthOk);

    server.shutdown_handle().send(()).unwrap();
    handle.await.unwrap();
}
