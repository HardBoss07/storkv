use crate::common;
use bytes::{Bytes, BytesMut};
use std::sync::Arc;
use storkv_proto::{Message, MessagePayload, Request, Response, WireChangeEvent};
use storkv_server::Server;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Duration as TokioDuration, timeout};

use super::support::{read_one_response, send_request, spawn_server};

#[tokio::test]
async fn test_watch_cdc_stream_over_socket() {
    let mut temp_db = common::TempDb::new("server_watch");
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

    let mut watch_stream = TcpStream::connect(addr).await.unwrap();
    let subscribe_response = send_request(&mut watch_stream, 10, Request::WatchSubscribe).await;
    assert_eq!(subscribe_response, Response::Empty);

    let mut writer_stream = TcpStream::connect(addr).await.unwrap();
    let put_response = send_request(
        &mut writer_stream,
        11,
        Request::Put {
            scope_id: 0,
            key: Bytes::from_static(b"stream_key"),
            val: Bytes::from_static(b"stream_val"),
        },
    )
    .await;
    assert_eq!(put_response, Response::Empty);

    let mut buffer = BytesMut::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let read = watch_stream.read(&mut chunk).await.unwrap();
    buffer.extend_from_slice(&chunk[..read]);

    let event_message = Message::decode(&mut buffer).unwrap().unwrap();
    match event_message.payload {
        MessagePayload::Response(Response::WatchEvent(WireChangeEvent::Put {
            scope_id,
            key,
            value,
            ..
        })) => {
            assert_eq!(scope_id, 0);
            assert_eq!(key, Bytes::from_static(b"stream_key"));
            assert_eq!(value, Bytes::from_static(b"stream_val"));
        }
        other => panic!("Expected WatchEvent(Put), got {:?}", other),
    }

    server.shutdown_handle().send(()).unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn test_five_concurrent_watch_subscribers_all_receive_events() {
    let (addr, server, _temp, handle) = spawn_server("server_watch_fanout").await;

    let mut watchers = Vec::new();
    for index in 0..5u64 {
        let mut stream = TcpStream::connect(&addr).await.unwrap();
        assert_eq!(
            send_request(&mut stream, index, Request::WatchSubscribe).await,
            Response::Empty
        );
        watchers.push(stream);
    }

    let mut writer = TcpStream::connect(&addr).await.unwrap();
    for index in 0..5u64 {
        let key = format!("fanout:{index}");
        assert_eq!(
            send_request(
                &mut writer,
                index + 100,
                Request::Put {
                    scope_id: 0,
                    key: Bytes::from(key),
                    val: Bytes::from_static(b"v"),
                },
            )
            .await,
            Response::Empty
        );
    }

    for (index, watcher) in watchers.iter_mut().enumerate() {
        let message = timeout(TokioDuration::from_secs(5), read_one_response(watcher))
            .await
            .unwrap_or_else(|_| panic!("watcher {index} received no event"));
        match message.payload {
            MessagePayload::Response(Response::WatchEvent(WireChangeEvent::Put { .. })) => {}
            other => panic!("watcher {index} expected a Put event, got {other:?}"),
        }
    }

    server.shutdown_handle().send(()).unwrap();
    handle.await.unwrap();
}
