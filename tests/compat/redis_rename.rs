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
async fn redis_rename_preserves_ttl_and_rejects_missing_source() {
    let server = spawn("compat_redis_rename").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    match client.command(&[b"RENAME", b"missing", b"target"]).await {
        Resp::Error(message) => assert!(message.contains("no such key"), "unexpected: {message}"),
        other => panic!("missing source must fail, got {other:?}"),
    }

    client
        .command(&[b"SET", b"source", b"value", b"PX", b"60000"])
        .await;
    assert_eq!(
        client.command(&[b"RENAME", b"source", b"dest"]).await,
        Resp::Status("OK".into())
    );
    assert_eq!(
        client.command(&[b"GET", b"dest"]).await,
        Resp::Bulk(Some(b"value".to_vec()))
    );
    match client.command(&[b"PTTL", b"dest"]).await {
        Resp::Integer(remaining) => assert!(remaining > 0 && remaining <= 60_000),
        other => panic!("rename must preserve the TTL, got {other:?}"),
    }
    assert_eq!(client.command(&[b"GET", b"source"]).await, Resp::Bulk(None));

    assert_eq!(
        client.command(&[b"RENAME", b"dest", b"dest"]).await,
        Resp::Status("OK".into())
    );
    assert_eq!(
        client.command(&[b"RENAMENX", b"dest", b"dest"]).await,
        Resp::Integer(0)
    );

    client.command(&[b"SET", b"occupied", b"keep"]).await;
    client.command(&[b"SET", b"free", b"move"]).await;
    assert_eq!(
        client.command(&[b"RENAMENX", b"free", b"occupied"]).await,
        Resp::Integer(0)
    );
    assert_eq!(
        client.command(&[b"GET", b"occupied"]).await,
        Resp::Bulk(Some(b"keep".to_vec()))
    );
    assert_eq!(
        client.command(&[b"GET", b"free"]).await,
        Resp::Bulk(Some(b"move".to_vec()))
    );
    assert_eq!(
        client.command(&[b"RENAMENX", b"free", b"fresh"]).await,
        Resp::Integer(1)
    );
    assert_eq!(
        client.command(&[b"GET", b"fresh"]).await,
        Resp::Bulk(Some(b"move".to_vec()))
    );
}

#[tokio::test]
async fn redis_extended_commands_inside_multi_and_command_metadata() {
    let server = spawn("compat_redis_extended_multi").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    client.command(&[b"SET", b"multi:a", b"1"]).await;
    client.command(&[b"SET", b"multi:b", b"2"]).await;
    let expiry = future_seconds(60);

    assert_eq!(client.command(&[b"MULTI"]).await, Resp::Status("OK".into()));
    for args in [
        &[b"MSETNX".as_slice(), b"multi:b", b"x", b"multi:c", b"3"][..],
        &[b"GETDEL".as_slice(), b"multi:a"][..],
        &[b"RENAME".as_slice(), b"multi:b", b"multi:d"][..],
        &[b"TYPE".as_slice(), b"multi:d"][..],
    ] {
        assert_eq!(client.command(args).await, Resp::Status("QUEUED".into()));
    }
    assert_eq!(
        client
            .command(&[b"EXPIREAT", b"multi:d", expiry.as_bytes()])
            .await,
        Resp::Status("QUEUED".into())
    );
    assert_eq!(
        client.command(&[b"EXPIRETIME", b"multi:d"]).await,
        Resp::Status("QUEUED".into())
    );
    assert_eq!(
        client.command(&[b"EXEC"]).await,
        Resp::Array(Some(vec![
            Resp::Integer(0),
            Resp::Bulk(Some(b"1".to_vec())),
            Resp::Status("OK".into()),
            Resp::Status("string".into()),
            Resp::Integer(1),
            Resp::Integer(expiry.parse().expect("seconds")),
        ]))
    );
    assert_eq!(
        client.command(&[b"GET", b"multi:a"]).await,
        Resp::Bulk(None)
    );
    assert_eq!(
        client.command(&[b"GET", b"multi:c"]).await,
        Resp::Bulk(None),
        "MSETNX must not write when any key exists"
    );
    assert_eq!(
        client.command(&[b"GET", b"multi:d"]).await,
        Resp::Bulk(Some(b"2".to_vec()))
    );

    let Resp::Integer(count) = client.command(&[b"COMMAND", b"COUNT"]).await else {
        panic!("COMMAND COUNT must return an integer");
    };
    let Resp::Array(Some(entries)) = client.command(&[b"COMMAND"]).await else {
        panic!("COMMAND must return an array");
    };
    assert_eq!(entries.len() as i64, count);

    for name in [
        b"getdel".as_slice(),
        b"getex",
        b"msetnx",
        b"rename",
        b"renamenx",
        b"type",
        b"expireat",
        b"pexpireat",
        b"expiretime",
        b"pexpiretime",
    ] {
        let entry = entries
            .iter()
            .find(|entry| {
                matches!(entry, Resp::Array(Some(fields))
                    if fields.first() == Some(&Resp::Bulk(Some(name.to_vec()))))
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing COMMAND entry for {}",
                    String::from_utf8_lossy(name)
                )
            });
        let Resp::Array(Some(fields)) = entry else {
            unreachable!();
        };
        assert!(fields.len() >= 6);
    }

    let rename = entries
        .iter()
        .find(|entry| {
            matches!(entry, Resp::Array(Some(fields))
                if fields.first() == Some(&Resp::Bulk(Some(b"rename".to_vec()))))
        })
        .expect("RENAME metadata");
    let Resp::Array(Some(fields)) = rename else {
        unreachable!();
    };
    assert_eq!(fields[1], Resp::Integer(3));
    assert_eq!(fields[3], Resp::Integer(1));
    assert_eq!(fields[4], Resp::Integer(2));
    assert_eq!(fields[5], Resp::Integer(1));
}
