use std::time::Duration;

use bytes::BytesMut;
use futures_util::{SinkExt, StreamExt};
use storkv_proto::{Message, MessagePayload, Request, Response, StorkvWireError, WireChangeEvent};
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use super::support;

#[tokio::test]
async fn websocket_watch_stream_receives_put_events() {
    let transports = support::spawn_all("transports_watch").await;
    let client = support::connect(&transports.ws_uri).await;
    let scope = support::make_scope(&client, "watch_scope").await;

    let mut watch = client.subscribe_watch().await.expect("subscribe watch");
    // A watch subscription owns its connection while waiting for events, so
    // writes use an independent client (the normal production pattern).
    let writer = support::connect(&transports.ws_uri).await;
    let writer_scope = scope.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        writer
            .put(&writer_scope, b"watched", b"value")
            .await
            .expect("watch put");
    });

    let event = tokio::time::timeout(Duration::from_secs(5), watch.next())
        .await
        .expect("watch timeout")
        .expect("watch transport")
        .expect("watch event");
    match event {
        WireChangeEvent::Put { key, .. } => {
            assert_eq!(key.as_ref(), b"watched");
        }
        other => panic!("expected put event, got {other:?}"),
    }
}

#[tokio::test]
async fn websocket_watcher_limit_is_enforced() {
    let transports = support::spawn_all("transports_ws_watch_cap").await;
    let (mut socket, _) = tokio_tungstenite::connect_async(&transports.ws_uri)
        .await
        .expect("ws connect");
    let mut buffer = BytesMut::new();

    for id in 0..16 {
        let frame = Message::new_request(id, Request::WatchSubscribe).encode();
        socket
            .send(WsMessage::Binary(frame))
            .await
            .expect("send watch");
        assert_eq!(
            read_ws_response(&mut socket, &mut buffer).await,
            Response::Empty
        );
    }

    let frame = Message::new_request(99, Request::WatchSubscribe).encode();
    socket
        .send(WsMessage::Binary(frame))
        .await
        .expect("send watch");
    match read_ws_response(&mut socket, &mut buffer).await {
        Response::Error(StorkvWireError::TooManySubscribers { max, .. }) => {
            assert_eq!(max, 16);
        }
        other => panic!("expected TooManySubscribers, got {other:?}"),
    }
}

async fn read_ws_response(
    socket: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    buffer: &mut BytesMut,
) -> Response {
    loop {
        if let Some(message) = Message::decode(buffer).expect("decode ws response") {
            match message.payload {
                MessagePayload::Response(response) => return response,
                _ => panic!("expected response payload"),
            }
        }
        let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .expect("ws read timeout")
            .expect("ws closed")
            .expect("ws read error");
        if let WsMessage::Binary(bytes) = frame {
            buffer.extend_from_slice(&bytes);
        }
    }
}
