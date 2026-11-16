use storkv_client::{AsyncClient, ClientError};
use storkv_proto::{Message, Request};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn test_connect_to_closed_port_returns_io_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    match AsyncClient::connect(&addr.to_string()).await {
        Err(ClientError::Io(_)) => {}
        Err(other) => panic!("expected Io error, got {other:?}"),
        Ok(_) => panic!("connecting to a closed port must fail"),
    }
}

#[tokio::test]
async fn test_malformed_server_reply_is_reported_as_protocol_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut chunk = [0u8; 4096];
            let _ = socket.read(&mut chunk).await;
            let reply = Message::new_request(99, Request::Ping).encode();
            let _ = socket.write_all(&reply).await;
        }
    });

    let client = AsyncClient::connect(&addr).await.unwrap();
    match client.ping().await {
        Err(ClientError::Protocol(_)) => {}
        other => panic!("expected Protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_server_closing_connection_surfaces_eof() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut chunk = [0u8; 4096];
            let _ = socket.read(&mut chunk).await;
            let _ = socket.shutdown().await;
        }
    });

    let client = AsyncClient::connect(&addr).await.unwrap();
    let error = client.ping().await.unwrap_err();
    assert!(
        matches!(error, ClientError::Io(_)),
        "expected Io, got {error:?}"
    );
}

#[tokio::test]
async fn test_oversized_server_frame_is_rejected() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut chunk = [0u8; 4096];
            let _ = socket.read(&mut chunk).await;
            let declared_length = (storkv_proto::MAX_FRAME_SIZE as u32) + 1;
            let _ = socket.write_all(&declared_length.to_be_bytes()).await;
        }
    });

    let client = AsyncClient::connect(&addr).await.unwrap();
    let error = client.ping().await.unwrap_err();
    assert!(
        matches!(
            error,
            ClientError::Server(storkv_proto::StorKvWireError::Protocol(_))
        ),
        "expected wire Protocol error, got {error:?}"
    );
}
