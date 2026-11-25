use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::support::{RedisClient, Resp, spawn};

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_millis()
}

#[tokio::test]
async fn redis_set_absolute_expiry() {
    let server = spawn("compat_redis_set_exat").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    let at_seconds = (unix_ms() / 1000 + 60).to_string();
    assert_eq!(
        client
            .command(&[b"SET", b"a", b"1", b"EXAT", at_seconds.as_bytes()])
            .await,
        Resp::Status("OK".into())
    );
    match client.command(&[b"PTTL", b"a"]).await {
        Resp::Integer(remaining) => assert!(remaining > 0 && remaining <= 60_000),
        other => panic!("unexpected PTTL reply: {other:?}"),
    }

    let at_millis = (unix_ms() + 60_000).to_string();
    assert_eq!(
        client
            .command(&[b"SET", b"b", b"2", b"PXAT", at_millis.as_bytes()])
            .await,
        Resp::Status("OK".into())
    );
    match client.command(&[b"PTTL", b"b"]).await {
        Resp::Integer(remaining) => assert!(remaining > 0 && remaining <= 60_000),
        other => panic!("unexpected PTTL reply: {other:?}"),
    }

    for options in [
        &[b"EXAT".as_slice(), at_seconds.as_bytes(), b"EX", b"10"][..],
        &[b"EX", b"10", b"PXAT", at_millis.as_bytes()],
        &[b"KEEPTTL", b"EXAT", at_seconds.as_bytes()],
    ] {
        let mut args: Vec<&[u8]> = vec![b"SET", b"invalid", b"value"];
        args.extend_from_slice(options);
        match client.command(&args).await {
            Resp::Error(message) => assert!(message.contains("syntax"), "unexpected: {message}"),
            other => panic!("conflicting expiry must fail, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn redis_getdel_and_getex() {
    let server = spawn("compat_redis_getdel_getex").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    assert_eq!(
        client.command(&[b"SET", b"gone", b"value"]).await,
        Resp::Status("OK".into())
    );
    assert_eq!(
        client.command(&[b"GETDEL", b"gone"]).await,
        Resp::Bulk(Some(b"value".to_vec()))
    );
    assert_eq!(
        client.command(&[b"GETDEL", b"gone"]).await,
        Resp::Bulk(None)
    );
    assert_eq!(
        client.command(&[b"EXISTS", b"gone"]).await,
        Resp::Integer(0)
    );

    assert_eq!(
        client
            .command(&[b"SET", b"keep", b"value", b"PX", b"60000"])
            .await,
        Resp::Status("OK".into())
    );
    assert_eq!(
        client.command(&[b"GETEX", b"keep", b"PERSIST"]).await,
        Resp::Bulk(Some(b"value".to_vec()))
    );
    assert_eq!(client.command(&[b"TTL", b"keep"]).await, Resp::Integer(-1));

    assert_eq!(
        client.command(&[b"GETEX", b"keep", b"EX", b"60"]).await,
        Resp::Bulk(Some(b"value".to_vec()))
    );
    match client.command(&[b"PTTL", b"keep"]).await {
        Resp::Integer(remaining) => assert!(remaining > 0 && remaining <= 60_000),
        other => panic!("unexpected PTTL reply: {other:?}"),
    }

    let at_seconds = (unix_ms() / 1000 + 60).to_string();
    assert_eq!(
        client.command(&[b"GETEX", b"keep", b"PX", b"60000"]).await,
        Resp::Bulk(Some(b"value".to_vec()))
    );
    assert_eq!(
        client
            .command(&[b"GETEX", b"keep", b"EXAT", at_seconds.as_bytes()])
            .await,
        Resp::Bulk(Some(b"value".to_vec()))
    );
    match client.command(&[b"EXPIRETIME", b"keep"]).await {
        Resp::Integer(seconds) => assert_eq!(seconds.to_string(), at_seconds),
        other => panic!("unexpected EXPIRETIME reply: {other:?}"),
    }

    let at_millis = (unix_ms() + 60_000).to_string();
    assert_eq!(
        client
            .command(&[b"GETEX", b"keep", b"PXAT", at_millis.as_bytes()])
            .await,
        Resp::Bulk(Some(b"value".to_vec()))
    );
    assert_eq!(
        client.command(&[b"GETEX", b"missing", b"EX", b"60"]).await,
        Resp::Bulk(None)
    );
    assert_eq!(
        client.command(&[b"EXISTS", b"missing"]).await,
        Resp::Integer(0)
    );

    match client
        .command(&[b"GETEX", b"keep", b"EX", b"10", b"PX", b"20"])
        .await
    {
        Resp::Error(message) => assert!(message.contains("syntax"), "unexpected: {message}"),
        other => panic!("conflicting GETEX options must fail, got {other:?}"),
    }
}

#[tokio::test]
async fn redis_msetnx_is_all_or_nothing() {
    let server = spawn("compat_redis_msetnx").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    assert_eq!(
        client.command(&[b"MSETNX", b"a", b"1", b"b", b"2"]).await,
        Resp::Integer(1)
    );
    assert_eq!(
        client.command(&[b"MGET", b"a", b"b"]).await,
        Resp::Array(Some(vec![
            Resp::Bulk(Some(b"1".to_vec())),
            Resp::Bulk(Some(b"2".to_vec())),
        ]))
    );

    assert_eq!(
        client
            .command(&[b"MSETNX", b"c", b"3", b"a", b"overwritten"])
            .await,
        Resp::Integer(0)
    );
    assert_eq!(client.command(&[b"GET", b"c"]).await, Resp::Bulk(None));
    assert_eq!(
        client.command(&[b"GET", b"a"]).await,
        Resp::Bulk(Some(b"1".to_vec()))
    );

    assert_eq!(
        client
            .command(&[b"MSETNX", b"d", b"first", b"d", b"last"])
            .await,
        Resp::Integer(1)
    );
    assert_eq!(
        client.command(&[b"GET", b"d"]).await,
        Resp::Bulk(Some(b"last".to_vec()))
    );

    match client.command(&[b"MSETNX", b"only-key"]).await {
        Resp::Error(message) => assert!(message.contains("wrong number"), "unexpected: {message}"),
        other => panic!("odd MSETNX arity must fail, got {other:?}"),
    }
}

#[tokio::test]
async fn redis_getdel_respects_short_ttl() {
    let server = spawn("compat_redis_getdel_ttl").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    client
        .command(&[b"SET", b"short", b"value", b"PX", b"1"])
        .await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        client.command(&[b"GETDEL", b"short"]).await,
        Resp::Bulk(None)
    );
}
