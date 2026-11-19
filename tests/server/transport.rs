#[cfg(unix)]
use crate::common;
use bytes::Bytes;
#[cfg(unix)]
use std::sync::Arc;
use storkv_proto::{MAX_FRAME_SIZE, Message, MessagePayload, Request, Response, StorkvWireError};
#[cfg(unix)]
use storkv_server::Server;
#[cfg(unix)]
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use super::support::{read_one_response, send_request, spawn_server};

#[cfg(unix)]
#[tokio::test]
async fn test_unix_domain_socket_transport() {
    use bytes::BytesMut;
    use storkv_server::ServerListener;

    let mut temp_db = common::TempDb::new("server_uds");
    let server = Arc::new(Server::with_db(Arc::new(temp_db.db.take().unwrap())));

    let sock_path = temp_db.path.join("admin.sock");
    let _ = std::fs::remove_file(&sock_path);
    let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

    let server_clone = server.clone();
    let handle = tokio::spawn(async move {
        server_clone
            .run_listener(ServerListener::Unix(listener), true)
            .await
            .unwrap();
    });

    let mut stream = tokio::net::UnixStream::connect(&sock_path).await.unwrap();
    let message = Message::new_request(1, Request::Ping);
    stream.write_all(&message.encode()).await.unwrap();
    let mut buffer = BytesMut::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await.unwrap();
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(response) = Message::decode(&mut buffer).unwrap() {
            assert_eq!(response.payload, MessagePayload::Response(Response::Pong));
            break;
        }
    }

    server.shutdown_handle().send(()).unwrap();
    handle.await.unwrap();
    let _ = std::fs::remove_file(&sock_path);
}

#[tokio::test]
async fn test_abrupt_disconnect_mid_frame_keeps_server_alive() {
    let (addr, server, _temp, handle) = spawn_server("server_abrupt").await;

    let message = Message::new_request(
        1,
        Request::Put {
            scope_id: 0,
            key: Bytes::from_static(b"never_completed"),
            val: Bytes::from_static(b"x"),
        },
    );
    let encoded = message.encode();
    {
        let mut stream = TcpStream::connect(&addr).await.unwrap();
        stream.write_all(&encoded[..10]).await.unwrap();
        stream.shutdown().await.unwrap();
    }

    let mut healthy = TcpStream::connect(&addr).await.unwrap();
    let response = send_request(&mut healthy, 2, Request::Ping).await;
    assert_eq!(response, Response::Pong);

    server.shutdown_handle().send(()).unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn test_oversized_and_garbage_frames_are_rejected_then_server_recovers() {
    let (addr, server, _temp, handle) = spawn_server("server_bad_frames").await;

    {
        let mut stream = TcpStream::connect(&addr).await.unwrap();
        stream
            .write_all(&((MAX_FRAME_SIZE as u32) + 1).to_be_bytes())
            .await
            .unwrap();
        let message = read_one_response(&mut stream).await;
        match message.payload {
            MessagePayload::Response(Response::Error(StorkvWireError::Protocol(_))) => {}
            other => panic!("expected Protocol error, got {other:?}"),
        }
    }

    {
        let mut stream = TcpStream::connect(&addr).await.unwrap();
        let body = b"NOTSTORKV-this-is-not-a-frame";
        stream
            .write_all(&(body.len() as u32).to_be_bytes())
            .await
            .unwrap();
        stream.write_all(body).await.unwrap();
        let message = read_one_response(&mut stream).await;
        match message.payload {
            MessagePayload::Response(Response::Error(StorkvWireError::Protocol(_))) => {}
            other => panic!("expected Protocol error, got {other:?}"),
        }
    }

    let mut stream = TcpStream::connect(&addr).await.unwrap();
    assert_eq!(
        send_request(&mut stream, 7, Request::Ping).await,
        Response::Pong
    );

    server.shutdown_handle().send(()).unwrap();
    handle.await.unwrap();
}
