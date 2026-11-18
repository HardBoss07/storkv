use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use storkv_proto::MAX_FRAME_SIZE;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use super::support;

fn tcp_address(uri: &str) -> &str {
    uri.strip_prefix("storkv://").expect("native uri")
}

#[tokio::test]
async fn slow_loris_partial_frame_does_not_block_healthy_clients() {
    let transports = support::spawn_all("transports_slow_loris").await;
    let address = tcp_address(&transports.tcp_uri);

    let mut slow = TcpStream::connect(address).await.expect("slow connect");
    // Announce a large frame, then dribble only a few bytes and stall.
    slow.write_all(&4096u32.to_be_bytes())
        .await
        .expect("length prefix");
    slow.write_all(&[0u8; 7]).await.expect("partial body");

    support::crud_roundtrip(&transports.tcp_uri).await;

    // The stalled connection must not have consumed server capacity.
    slow.write_all(&[0u8; 7]).await.expect("partial body again");
    drop(slow);
}

#[tokio::test]
async fn oversized_native_frame_closes_only_the_offending_connection() {
    let transports = support::spawn_all("transports_oversized").await;
    let address = tcp_address(&transports.tcp_uri);

    let mut offender = TcpStream::connect(address).await.expect("connect");
    let oversized = u32::try_from(MAX_FRAME_SIZE + 1).expect("frame size fits u32");
    offender
        .write_all(&oversized.to_be_bytes())
        .await
        .expect("write length");
    let mut scratch = [0u8; 16];
    match tokio::time::timeout(Duration::from_secs(5), offender.read(&mut scratch)).await {
        Ok(Ok(0)) | Ok(Err(_)) => {}
        Ok(Ok(read)) => assert!(
            read >= 4,
            "oversized frame must yield a length-prefixed error or close, got {read} bytes"
        ),
        Err(_) => panic!("server did not answer an oversized frame"),
    }

    support::crud_roundtrip(&transports.tcp_uri).await;
}

#[tokio::test]
async fn malformed_websocket_json_and_binary_frames_keep_the_session_alive() {
    let transports = support::spawn_all("transports_ws_adversarial").await;

    let (mut socket, _) = tokio_tungstenite::connect_async(&transports.ws_uri)
        .await
        .expect("ws connect");

    socket
        .send(WsMessage::Text("this is not json".into()))
        .await
        .expect("send malformed json");
    let reply = next_text(&mut socket).await;
    assert!(
        reply.contains("invalid_json") || reply.contains("error"),
        "unexpected JSON error reply: {reply}"
    );

    socket
        .send(WsMessage::Binary(vec![0xFF; 32].into()))
        .await
        .expect("send malformed binary");
    // Either a binary error frame or a clean close is acceptable; the session
    // must not panic the listener.
    match tokio::time::timeout(Duration::from_secs(5), socket.next()).await {
        Ok(Some(Ok(_))) | Ok(Some(Err(_))) | Ok(None) => {}
        Err(_) => panic!("server did not answer a malformed binary frame"),
    }

    let (mut healthy, _) = tokio_tungstenite::connect_async(&transports.ws_uri)
        .await
        .expect("second ws connect");
    healthy
        .send(WsMessage::Text(
            "{\"id\":7,\"op\":\"ping\"}".to_string().into(),
        ))
        .await
        .expect("send ping");
    let pong = next_text(&mut healthy).await;
    assert!(
        pong.contains("\"ok\":true"),
        "unexpected ping reply: {pong}"
    );
}

async fn next_text<S>(socket: &mut S) -> String
where
    S: futures_util::Stream<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
{
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .expect("websocket timeout")
            .expect("websocket closed")
            .expect("websocket error");
        match message {
            WsMessage::Text(text) => return text.to_string(),
            WsMessage::Binary(bytes) => return String::from_utf8_lossy(&bytes).to_string(),
            _ => continue,
        }
    }
}
