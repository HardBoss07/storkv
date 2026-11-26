use std::collections::HashSet;

use crate::support::{RedisClient, Resp, spawn};

fn bulk(value: &[u8]) -> Resp {
    Resp::Bulk(Some(value.to_vec()))
}

fn error_contains(response: Resp, expected: &str) {
    match response {
        Resp::Error(message) => {
            assert!(message.contains(expected), "unexpected error: {message}");
        }
        other => panic!("expected an error, got {other:?}"),
    }
}
#[tokio::test]
async fn redis_collections_hide_internal_keys() {
    let server = spawn("compat_redis_collection_keys").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    client.command(&[b"SET", b"plain", b"value"]).await;
    client
        .command(&[b"HSET", b"hash", b"field", b"value"])
        .await;
    client.command(&[b"SADD", b"set", b"member"]).await;

    assert_eq!(
        client.command(&[b"TYPE", b"hash"]).await,
        Resp::Status("hash".into())
    );
    assert_eq!(
        client.command(&[b"TYPE", b"set"]).await,
        Resp::Status("set".into())
    );
    assert_eq!(
        client.command(&[b"TYPE", b"plain"]).await,
        Resp::Status("string".into())
    );
    assert_eq!(client.command(&[b"DBSIZE"]).await, Resp::Integer(3));

    let keys = match client.command(&[b"KEYS", b"*"]).await {
        Resp::Array(Some(values)) => values,
        other => panic!("unexpected KEYS reply: {other:?}"),
    };
    let keys: HashSet<Vec<u8>> = keys
        .into_iter()
        .map(|key| match key {
            Resp::Bulk(Some(bytes)) => bytes,
            other => panic!("unexpected KEYS entry: {other:?}"),
        })
        .collect();
    assert_eq!(
        keys,
        HashSet::from([b"plain".to_vec(), b"hash".to_vec(), b"set".to_vec()])
    );
    assert!(keys.iter().all(|key| key.first() != Some(&1)));

    match client.command(&[b"SCAN", b"0"]).await {
        Resp::Array(Some(values)) => match &values[1] {
            Resp::Array(Some(entries)) => {
                assert_eq!(entries.len(), 3);
                assert!(entries.iter().all(|entry| matches!(
                    entry,
                    Resp::Bulk(Some(key)) if key.first() != Some(&1)
                )));
            }
            other => panic!("unexpected SCAN page: {other:?}"),
        },
        other => panic!("unexpected SCAN reply: {other:?}"),
    }

    error_contains(
        client
            .command(&[b"HSET", b"plain", b"field", b"value"])
            .await,
        "WRONGTYPE",
    );
    error_contains(
        client.command(&[b"SADD", b"plain", b"member"]).await,
        "WRONGTYPE",
    );
    error_contains(
        client.command(&[b"SADD", b"hash", b"member"]).await,
        "WRONGTYPE",
    );
    error_contains(
        client.command(&[b"HGET", b"set", b"field"]).await,
        "WRONGTYPE",
    );
}

#[tokio::test]
async fn redis_hincrby_is_atomic_under_contention() {
    let server = spawn("compat_redis_collection_contention").await;
    let mut first = RedisClient::connect(&server.redis_addr).await;
    let mut second = RedisClient::connect(&server.redis_addr).await;

    let task = tokio::spawn(async move {
        for _ in 0..20 {
            increment(&mut first).await;
        }
    });
    for _ in 0..20 {
        increment(&mut second).await;
    }
    task.await.expect("increment task");

    assert_eq!(client_value(&server.redis_addr).await, bulk(b"40"));
}

async fn increment(client: &mut RedisClient) {
    loop {
        match client.command(&[b"HINCRBY", b"shared", b"n", b"1"]).await {
            Resp::Error(message) if message.contains("write conflict") => continue,
            Resp::Integer(_) => return,
            other => panic!("unexpected HINCRBY reply: {other:?}"),
        }
    }
}

async fn client_value(address: &str) -> Resp {
    let mut client = RedisClient::connect(address).await;
    client.command(&[b"HGET", b"shared", b"n"]).await
}
