use bytes::BytesMut;
use std::sync::Arc;
use storkv_proto::{Message, MessagePayload, Request, Response};
use storkv_server::Server;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Duration as TokioDuration, timeout};

pub(super) async fn send_request(stream: &mut TcpStream, req_id: u64, req: Request) -> Response {
    let message = Message::new_request(req_id, req);
    stream.write_all(&message.encode()).await.unwrap();

    let mut buffer = BytesMut::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "Server closed connection unexpectedly");
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(response_message) = Message::decode(&mut buffer).unwrap() {
            match response_message.payload {
                MessagePayload::Response(response) => return response,
                _ => panic!("Expected response payload"),
            }
        }
    }
}

pub(super) async fn spawn_server(
    prefix: &str,
) -> (
    String,
    Arc<Server>,
    crate::common::TempDb,
    tokio::task::JoinHandle<()>,
) {
    let mut temp_db = crate::common::TempDb::new(prefix);
    let server = Arc::new(Server::with_db(Arc::new(temp_db.db.take().unwrap())));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let server_clone = server.clone();
    let handle = tokio::spawn(async move {
        server_clone
            .run_tcp_listener(listener, false)
            .await
            .unwrap();
    });
    (addr, server, temp_db, handle)
}

pub(super) async fn read_one_response(stream: &mut TcpStream) -> Message {
    let mut buffer = BytesMut::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = timeout(TokioDuration::from_secs(5), stream.read(&mut chunk))
            .await
            .expect("server response timed out")
            .expect("read failed");
        assert!(read > 0, "server closed the connection without a response");
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(message) = Message::decode(&mut buffer).expect("decode response") {
            return message;
        }
    }
}
