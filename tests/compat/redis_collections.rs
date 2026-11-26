use crate::support::{RedisClient, Resp, spawn};

fn bulk(value: &[u8]) -> Resp {
    Resp::Bulk(Some(value.to_vec()))
}

fn bulk_set(values: &[&[u8]]) -> Resp {
    Resp::Array(Some(values.iter().map(|value| bulk(value)).collect()))
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
async fn redis_hash_commands() {
    let server = spawn("compat_redis_hashes").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    assert_eq!(
        client
            .command(&[b"HSET", b"person", b"name", b"alice", b"age", b"30"])
            .await,
        Resp::Integer(2)
    );
    assert_eq!(
        client
            .command(&[b"HSET", b"person", b"name", b"bob", b"city", b"paris"])
            .await,
        Resp::Integer(1)
    );
    assert_eq!(
        client.command(&[b"HGET", b"person", b"name"]).await,
        bulk(b"bob")
    );
    assert_eq!(
        client.command(&[b"HGET", b"person", b"missing"]).await,
        Resp::Bulk(None)
    );
    assert_eq!(
        client
            .command(&[b"HMGET", b"person", b"name", b"missing", b"age"])
            .await,
        Resp::Array(Some(vec![bulk(b"bob"), Resp::Bulk(None), bulk(b"30")]))
    );
    assert_eq!(
        client.command(&[b"HEXISTS", b"person", b"name"]).await,
        Resp::Integer(1)
    );
    assert_eq!(
        client.command(&[b"HEXISTS", b"person", b"nope"]).await,
        Resp::Integer(0)
    );
    assert_eq!(
        client.command(&[b"HLEN", b"person"]).await,
        Resp::Integer(3)
    );
    assert_eq!(
        client.command(&[b"HKEYS", b"person"]).await,
        bulk_set(&[b"age", b"city", b"name"])
    );
    assert_eq!(
        client.command(&[b"HVALS", b"person"]).await,
        bulk_set(&[b"30", b"paris", b"bob"])
    );
    assert_eq!(
        client.command(&[b"HGETALL", b"person"]).await,
        bulk_set(&[b"age", b"30", b"city", b"paris", b"name", b"bob"])
    );
    assert_eq!(
        client
            .command(&[b"HDEL", b"person", b"name", b"missing"])
            .await,
        Resp::Integer(1)
    );
    assert_eq!(
        client.command(&[b"HLEN", b"person"]).await,
        Resp::Integer(2)
    );
    assert_eq!(
        client.command(&[b"HDEL", b"person", b"age", b"city"]).await,
        Resp::Integer(2)
    );
    assert_eq!(
        client.command(&[b"TYPE", b"person"]).await,
        Resp::Status("none".into())
    );
    assert_eq!(
        client.command(&[b"HLEN", b"person"]).await,
        Resp::Integer(0)
    );

    error_contains(
        client.command(&[b"HSET", b"broken", b"field"]).await,
        "wrong number",
    );
    error_contains(client.command(&[b"HGET", b"broken"]).await, "wrong number");
}

#[tokio::test]
async fn redis_hash_increment_and_errors() {
    let server = spawn("compat_redis_hincrby").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    assert_eq!(
        client.command(&[b"HINCRBY", b"counter", b"n", b"5"]).await,
        Resp::Integer(5)
    );
    assert_eq!(
        client.command(&[b"HINCRBY", b"counter", b"n", b"3"]).await,
        Resp::Integer(8)
    );
    assert_eq!(
        client.command(&[b"TYPE", b"counter"]).await,
        Resp::Status("hash".into())
    );
    assert_eq!(
        client
            .command(&[b"HINCRBY", b"counter", b"n", b"-10"])
            .await,
        Resp::Integer(-2)
    );
    assert_eq!(
        client.command(&[b"HLEN", b"counter"]).await,
        Resp::Integer(1)
    );

    client
        .command(&[b"HSET", b"counter", b"word", b"text"])
        .await;
    error_contains(
        client
            .command(&[b"HINCRBY", b"counter", b"word", b"1"])
            .await,
        "not an integer",
    );
    client
        .command(&[b"HSET", b"limits", b"high", b"9223372036854775807"])
        .await;
    error_contains(
        client
            .command(&[b"HINCRBY", b"limits", b"high", b"1"])
            .await,
        "overflow",
    );
    error_contains(
        client
            .command(&[b"HINCRBY", b"counter", b"n", b"abc"])
            .await,
        "not an integer",
    );
}

#[tokio::test]
async fn redis_set_commands() {
    let server = spawn("compat_redis_sets").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    assert_eq!(
        client.command(&[b"SADD", b"tags", b"a", b"b", b"c"]).await,
        Resp::Integer(3)
    );
    assert_eq!(
        client.command(&[b"SADD", b"tags", b"b", b"d"]).await,
        Resp::Integer(1)
    );
    assert_eq!(client.command(&[b"SCARD", b"tags"]).await, Resp::Integer(4));
    assert_eq!(
        client.command(&[b"SISMEMBER", b"tags", b"a"]).await,
        Resp::Integer(1)
    );
    assert_eq!(
        client.command(&[b"SISMEMBER", b"tags", b"z"]).await,
        Resp::Integer(0)
    );
    assert_eq!(
        client.command(&[b"SMEMBERS", b"tags"]).await,
        bulk_set(&[b"a", b"b", b"c", b"d"])
    );
    assert_eq!(
        client.command(&[b"SREM", b"tags", b"a", b"z"]).await,
        Resp::Integer(1)
    );
    assert_eq!(
        client.command(&[b"TYPE", b"tags"]).await,
        Resp::Status("set".into())
    );
    assert_eq!(
        client.command(&[b"SREM", b"tags", b"b", b"c", b"d"]).await,
        Resp::Integer(3)
    );
    assert_eq!(
        client.command(&[b"TYPE", b"tags"]).await,
        Resp::Status("none".into())
    );
    assert_eq!(client.command(&[b"SCARD", b"tags"]).await, Resp::Integer(0));
    assert_eq!(
        client.command(&[b"SADD", b"tags", b"a"]).await,
        Resp::Integer(1)
    );
}

#[tokio::test]
async fn redis_collections_inside_multi() {
    let server = spawn("compat_redis_collection_multi").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    assert_eq!(client.command(&[b"MULTI"]).await, Resp::Status("OK".into()));
    assert_eq!(
        client.command(&[b"HSET", b"cart", b"apple", b"2"]).await,
        Resp::Status("QUEUED".into())
    );
    assert_eq!(
        client.command(&[b"HINCRBY", b"cart", b"apple", b"3"]).await,
        Resp::Status("QUEUED".into())
    );
    assert_eq!(
        client.command(&[b"SADD", b"bag", b"red"]).await,
        Resp::Status("QUEUED".into())
    );
    assert_eq!(
        client.command(&[b"EXEC"]).await,
        Resp::Array(Some(vec![
            Resp::Integer(1),
            Resp::Integer(5),
            Resp::Integer(1)
        ]))
    );
    assert_eq!(
        client.command(&[b"HGET", b"cart", b"apple"]).await,
        bulk(b"5")
    );
    assert_eq!(
        client.command(&[b"SMEMBERS", b"bag"]).await,
        bulk_set(&[b"red"])
    );
}
