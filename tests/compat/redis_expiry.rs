use std::time::{SystemTime, UNIX_EPOCH};

use crate::support::{RedisClient, Resp, spawn};

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_millis()
}

fn future_seconds(offset: u128) -> String {
    (unix_ms() / 1000 + offset).to_string()
}

#[tokio::test]
async fn redis_type_and_absolute_expiry_flags() {
    let server = spawn("compat_redis_expireat").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    assert_eq!(
        client.command(&[b"TYPE", b"missing"]).await,
        Resp::Status("none".into())
    );
    client.command(&[b"SET", b"k", b"v"]).await;
    assert_eq!(
        client.command(&[b"TYPE", b"k"]).await,
        Resp::Status("string".into())
    );

    let first = future_seconds(120);
    assert_eq!(
        client.command(&[b"EXPIREAT", b"k", first.as_bytes()]).await,
        Resp::Integer(1)
    );
    assert_eq!(
        client.command(&[b"EXPIRETIME", b"k"]).await,
        Resp::Integer(first.parse().expect("seconds"))
    );

    assert_eq!(
        client
            .command(&[b"EXPIREAT", b"k", first.as_bytes(), b"NX"])
            .await,
        Resp::Integer(0)
    );
    let later = future_seconds(240);
    assert_eq!(
        client
            .command(&[b"EXPIREAT", b"k", later.as_bytes(), b"XX"])
            .await,
        Resp::Integer(1)
    );
    assert_eq!(
        client
            .command(&[b"EXPIREAT", b"k", first.as_bytes(), b"GT"])
            .await,
        Resp::Integer(0)
    );
    let latest = future_seconds(360);
    assert_eq!(
        client
            .command(&[b"EXPIREAT", b"k", latest.as_bytes(), b"GT"])
            .await,
        Resp::Integer(1)
    );
    assert_eq!(
        client
            .command(&[b"EXPIREAT", b"k", later.as_bytes(), b"LT"])
            .await,
        Resp::Integer(1)
    );

    client.command(&[b"PERSIST", b"k"]).await;
    assert_eq!(
        client
            .command(&[b"EXPIREAT", b"k", later.as_bytes(), b"GT"])
            .await,
        Resp::Integer(0),
        "GT must fail when the key has no expiry"
    );
    assert_eq!(
        client
            .command(&[b"EXPIREAT", b"k", later.as_bytes(), b"LT"])
            .await,
        Resp::Integer(1),
        "LT must succeed when the key has no expiry"
    );

    let past = (unix_ms() / 1000).saturating_sub(10).to_string();
    assert_eq!(
        client.command(&[b"PEXPIREAT", b"k", past.as_bytes()]).await,
        Resp::Integer(1)
    );
    assert_eq!(client.command(&[b"EXISTS", b"k"]).await, Resp::Integer(0));
    assert_eq!(
        client.command(&[b"PEXPIREAT", b"k", past.as_bytes()]).await,
        Resp::Integer(0)
    );
}

#[tokio::test]
async fn redis_expiretime_reports_absolute_timestamps() {
    let server = spawn("compat_redis_expiretime").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    assert_eq!(
        client.command(&[b"EXPIRETIME", b"missing"]).await,
        Resp::Integer(-2)
    );
    assert_eq!(
        client.command(&[b"PEXPIRETIME", b"missing"]).await,
        Resp::Integer(-2)
    );

    client.command(&[b"SET", b"permanent", b"v"]).await;
    assert_eq!(
        client.command(&[b"EXPIRETIME", b"permanent"]).await,
        Resp::Integer(-1)
    );
    assert_eq!(
        client.command(&[b"PEXPIRETIME", b"permanent"]).await,
        Resp::Integer(-1)
    );

    let now = unix_ms();
    client
        .command(&[b"SET", b"timed", b"v", b"PX", b"60000"])
        .await;
    match client.command(&[b"EXPIRETIME", b"timed"]).await {
        Resp::Integer(seconds) => {
            let expected = ((now + 60_000) / 1000) as i64;
            assert!(
                (seconds - expected).abs() <= 2,
                "unexpected seconds: {seconds}"
            );
        }
        other => panic!("unexpected EXPIRETIME reply: {other:?}"),
    }
    match client.command(&[b"PEXPIRETIME", b"timed"]).await {
        Resp::Integer(millis) => {
            let expected = (now + 60_000) as i64;
            assert!(
                (millis - expected).abs() <= 2_000,
                "unexpected milliseconds: {millis}"
            );
        }
        other => panic!("unexpected PEXPIRETIME reply: {other:?}"),
    }
}
