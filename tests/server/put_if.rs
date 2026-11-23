use bytes::Bytes;
use storkv_proto::{
    Request, Response, WireDurability, WirePutCondition, WirePutIfResult, WireRetention,
    WireScopeOptions,
};
use tokio::net::TcpStream;

use super::support::{send_request, spawn_server};

fn options() -> WireScopeOptions {
    WireScopeOptions {
        durability: WireDurability::Buffered {
            sync_interval: std::time::Duration::from_millis(100),
        },
        retention: WireRetention::Permanent,
        prefix_bloom_len: None,
        min_ttl: None,
        max_ttl: None,
        allow_permanent: true,
        active_ttl_tracking: false,
    }
}

#[tokio::test]
async fn put_if_round_trip_over_native_transport() {
    let (addr, _server, _temp, handle) = spawn_server("server_put_if").await;
    let mut stream = TcpStream::connect(&addr).await.unwrap();

    let scope_id = match send_request(
        &mut stream,
        1,
        Request::CreateScope {
            name: "cas".into(),
            options: options(),
        },
    )
    .await
    {
        Response::Scope(handle) => handle.scope_id,
        other => panic!("unexpected scope response: {other:?}"),
    };

    let first = send_request(
        &mut stream,
        2,
        Request::PutIf {
            scope_id,
            key: Bytes::from_static(b"key"),
            condition: WirePutCondition::Absent,
            val: Bytes::from_static(b"v1"),
            ttl: None,
        },
    )
    .await;
    let seq = match first {
        Response::PutIf(WirePutIfResult::Applied { seq }) => seq,
        other => panic!("expected applied, got {other:?}"),
    };

    let conflict = send_request(
        &mut stream,
        3,
        Request::PutIf {
            scope_id,
            key: Bytes::from_static(b"key"),
            condition: WirePutCondition::Absent,
            val: Bytes::from_static(b"v2"),
            ttl: None,
        },
    )
    .await;
    assert_eq!(
        conflict,
        Response::PutIf(WirePutIfResult::Conflict {
            current_seq: Some(seq),
            current_value: Some(Bytes::from_static(b"v1")),
        })
    );

    let applied = send_request(
        &mut stream,
        4,
        Request::PutIf {
            scope_id,
            key: Bytes::from_static(b"key"),
            condition: WirePutCondition::ExpectedSeq(seq),
            val: Bytes::from_static(b"v2"),
            ttl: Some(std::time::Duration::from_secs(60)),
        },
    )
    .await;
    assert!(matches!(
        applied,
        Response::PutIf(WirePutIfResult::Applied { .. })
    ));

    let read = send_request(
        &mut stream,
        5,
        Request::Get {
            scope_id,
            key: Bytes::from_static(b"key"),
        },
    )
    .await;
    assert_eq!(read, Response::Value(Some(Bytes::from_static(b"v2"))));

    handle.abort();
}
