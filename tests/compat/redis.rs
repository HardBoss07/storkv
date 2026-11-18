use crate::support::{RedisClient, Resp, spawn};

#[tokio::test]
async fn redis_string_commands_and_ttl() {
    let server = spawn("compat_redis_strings").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    assert_eq!(
        client.command(&[b"PING"]).await,
        Resp::Status("PONG".into())
    );
    assert_eq!(
        client.command(&[b"ECHO", b"hello"]).await,
        Resp::Bulk(Some(b"hello".to_vec()))
    );
    assert_eq!(
        client.command(&[b"SET", b"greeting", b"hello"]).await,
        Resp::Status("OK".into())
    );
    assert_eq!(
        client.command(&[b"GET", b"greeting"]).await,
        Resp::Bulk(Some(b"hello".to_vec()))
    );
    assert_eq!(
        client.command(&[b"EXISTS", b"greeting", b"missing"]).await,
        Resp::Integer(1)
    );
    assert_eq!(
        client.command(&[b"STRLEN", b"greeting"]).await,
        Resp::Integer(5)
    );
    assert_eq!(
        client.command(&[b"APPEND", b"greeting", b"!"]).await,
        Resp::Integer(6)
    );
    assert_eq!(
        client.command(&[b"GET", b"greeting"]).await,
        Resp::Bulk(Some(b"hello!".to_vec()))
    );
    assert_eq!(
        client.command(&[b"GET", b"missing"]).await,
        Resp::Bulk(None)
    );

    client.command(&[b"SET", b"counter", b"10"]).await;
    assert_eq!(
        client.command(&[b"INCR", b"counter"]).await,
        Resp::Integer(11)
    );
    assert_eq!(
        client.command(&[b"INCRBY", b"counter", b"9"]).await,
        Resp::Integer(20)
    );
    assert_eq!(
        client.command(&[b"DECRBY", b"counter", b"5"]).await,
        Resp::Integer(15)
    );
    assert_eq!(
        client.command(&[b"DECR", b"counter"]).await,
        Resp::Integer(14)
    );

    assert_eq!(
        client
            .command(&[b"SET", b"session", b"token", b"PX", b"60000"])
            .await,
        Resp::Status("OK".into())
    );
    match client.command(&[b"PTTL", b"session"]).await {
        Resp::Integer(remaining) => assert!(remaining > 0 && remaining <= 60_000),
        other => panic!("unexpected PTTL reply: {other:?}"),
    }
    assert_eq!(
        client.command(&[b"PERSIST", b"session"]).await,
        Resp::Integer(1)
    );
    assert_eq!(
        client.command(&[b"TTL", b"session"]).await,
        Resp::Integer(-1)
    );
    assert_eq!(
        client.command(&[b"TTL", b"missing"]).await,
        Resp::Integer(-2)
    );
}

#[tokio::test]
async fn redis_conditional_writes_and_bulk_commands() {
    let server = spawn("compat_redis_bulk").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    assert_eq!(
        client.command(&[b"SETNX", b"lock", b"owner"]).await,
        Resp::Integer(1)
    );
    assert_eq!(
        client.command(&[b"SETNX", b"lock", b"intruder"]).await,
        Resp::Integer(0)
    );
    assert_eq!(
        client.command(&[b"SET", b"lock", b"other", b"NX"]).await,
        Resp::Bulk(None)
    );
    assert_eq!(
        client.command(&[b"SET", b"lock", b"rotated", b"GET"]).await,
        Resp::Bulk(Some(b"owner".to_vec()))
    );
    assert_eq!(
        client.command(&[b"SET", b"absent", b"value", b"XX"]).await,
        Resp::Bulk(None)
    );

    assert_eq!(
        client
            .command(&[b"MSET", b"a", b"1", b"b", b"2", b"c", b"3"])
            .await,
        Resp::Status("OK".into())
    );
    assert_eq!(
        client.command(&[b"MGET", b"a", b"b", b"nope"]).await,
        Resp::Array(Some(vec![
            Resp::Bulk(Some(b"1".to_vec())),
            Resp::Bulk(Some(b"2".to_vec())),
            Resp::Bulk(None),
        ]))
    );
    assert_eq!(
        client.command(&[b"DEL", b"a", b"b", b"nope"]).await,
        Resp::Integer(2)
    );
    assert_eq!(client.command(&[b"UNLINK", b"c"]).await, Resp::Integer(1));
    assert_eq!(client.command(&[b"DBSIZE"]).await, Resp::Integer(1));
}

#[tokio::test]
async fn redis_select_isolates_logical_databases() {
    let server = spawn("compat_redis_select").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    client.command(&[b"SET", b"shared", b"zero"]).await;
    assert_eq!(
        client.command(&[b"SELECT", b"1"]).await,
        Resp::Status("OK".into())
    );
    assert_eq!(client.command(&[b"GET", b"shared"]).await, Resp::Bulk(None));
    client.command(&[b"SET", b"shared", b"one"]).await;
    assert_eq!(
        client.command(&[b"SELECT", b"0"]).await,
        Resp::Status("OK".into())
    );
    assert_eq!(
        client.command(&[b"GET", b"shared"]).await,
        Resp::Bulk(Some(b"zero".to_vec()))
    );

    client.command(&[b"FLUSHALL"]).await;
    assert_eq!(client.command(&[b"DBSIZE"]).await, Resp::Integer(0));
}

#[tokio::test]
async fn redis_multi_exec_and_abort() {
    let server = spawn("compat_redis_multi").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    assert_eq!(client.command(&[b"MULTI"]).await, Resp::Status("OK".into()));
    assert_eq!(
        client.command(&[b"SET", b"tx_key", b"tx_value"]).await,
        Resp::Status("QUEUED".into())
    );
    assert_eq!(
        client.command(&[b"INCR", b"tx_counter"]).await,
        Resp::Status("QUEUED".into())
    );
    assert_eq!(
        client.command(&[b"EXEC"]).await,
        Resp::Array(Some(vec![Resp::Status("OK".into()), Resp::Integer(1),]))
    );
    assert_eq!(
        client.command(&[b"GET", b"tx_key"]).await,
        Resp::Bulk(Some(b"tx_value".to_vec()))
    );

    client.command(&[b"MULTI"]).await;
    client.command(&[b"BOGUS"]).await;
    match client.command(&[b"EXEC"]).await {
        Resp::Error(message) => assert!(message.starts_with("EXECABORT")),
        other => panic!("unexpected EXEC reply: {other:?}"),
    }
    assert_eq!(
        client.command(&[b"DISCARD"]).await,
        Resp::Error("ERR DISCARD without MULTI".into())
    );
}

#[tokio::test]
async fn redis_watch_aborts_exec_when_key_changes() {
    let server = spawn("compat_redis_watch").await;
    let mut watcher = RedisClient::connect(&server.redis_addr).await;
    let mut writer = RedisClient::connect(&server.redis_addr).await;

    writer.command(&[b"SET", b"watched", b"before"]).await;
    assert_eq!(
        watcher.command(&[b"WATCH", b"watched"]).await,
        Resp::Status("OK".into())
    );

    writer.command(&[b"SET", b"watched", b"after"]).await;

    assert_eq!(
        watcher.command(&[b"MULTI"]).await,
        Resp::Status("OK".into())
    );
    assert_eq!(
        watcher.command(&[b"GET", b"watched"]).await,
        Resp::Status("QUEUED".into())
    );
    assert_eq!(
        watcher
            .command(&[b"SET", b"tx_after_watch", b"value"])
            .await,
        Resp::Status("QUEUED".into())
    );
    assert_eq!(watcher.command(&[b"EXEC"]).await, Resp::Array(None));
    assert_eq!(
        watcher.command(&[b"GET", b"tx_after_watch"]).await,
        Resp::Bulk(None)
    );

    watcher.command(&[b"WATCH", b"watched"]).await;
    watcher.command(&[b"UNWATCH"]).await;
    writer
        .command(&[b"SET", b"watched", b"changed-again"])
        .await;
    watcher.command(&[b"MULTI"]).await;
    watcher.command(&[b"SET", b"unwatched_tx", b"value"]).await;
    assert_eq!(
        watcher.command(&[b"EXEC"]).await,
        Resp::Array(Some(vec![Resp::Status("OK".into())]))
    );
}

#[tokio::test]
async fn redis_keys_scan_and_resp3() {
    let server = spawn("compat_redis_scan").await;
    let mut client = RedisClient::connect(&server.redis_addr).await;

    client
        .command(&[
            b"MSET", b"user:1", b"a", b"user:2", b"b", b"user:3", b"c", b"other", b"d",
        ])
        .await;

    let keys = client.command(&[b"KEYS", b"user:*"]).await;
    let Resp::Array(Some(keys)) = keys else {
        panic!("unexpected KEYS reply: {keys:?}");
    };
    assert_eq!(keys.len(), 3);

    let mut cursor = b"0".to_vec();
    let mut scanned = Vec::new();
    loop {
        let reply = client
            .command(&[b"SCAN", &cursor, b"MATCH", b"user:*", b"COUNT", b"2"])
            .await;
        let Resp::Array(Some(parts)) = reply else {
            panic!("unexpected SCAN reply: {reply:?}");
        };
        let mut parts = parts.into_iter();
        let Some(Resp::Bulk(Some(next))) = parts.next() else {
            panic!("missing SCAN cursor");
        };
        let Some(Resp::Array(Some(page))) = parts.next() else {
            panic!("missing SCAN page");
        };
        for key in page {
            if let Resp::Bulk(Some(key)) = key {
                scanned.push(key);
            }
        }
        cursor = next;
        if cursor == b"0" {
            break;
        }
    }
    scanned.sort();
    assert_eq!(
        scanned,
        vec![b"user:1".to_vec(), b"user:2".to_vec(), b"user:3".to_vec()]
    );

    let hello = client.command(&[b"HELLO", b"3"]).await;
    let Resp::Map(pairs) = hello else {
        panic!("unexpected HELLO reply: {hello:?}");
    };
    assert!(pairs.iter().any(|(key, value)| {
        key == &Resp::Bulk(Some(b"proto".to_vec())) && value == &Resp::Integer(3)
    }));
    assert_eq!(client.command(&[b"GET", b"missing"]).await, Resp::Null);
}
