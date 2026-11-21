use serde_json::json;
use storkv_core::{Db, ScopeHandle, ScopeOptions};
use storkv_server::ReplicationRole;

use crate::support::{MemcachedClient, RedisClient, Resp, http_post, spawn_with_role};

fn create_scope(db: &Db, name: &str) -> ScopeHandle {
    db.create_scope(name.to_string(), ScopeOptions::default())
        .expect("create scope")
}

fn scope_handle(db: &Db, name: &str) -> ScopeHandle {
    let scope_id = *db
        .inner()
        .scope_names
        .read()
        .get(name)
        .expect("scope exists");
    ScopeHandle {
        scope_id,
        name: name.to_string(),
    }
}

fn has_scope(db: &Db, name: &str) -> bool {
    db.inner().scope_names.read().contains_key(name)
}

fn map_value<'a>(response: &'a Resp, key: &str) -> Option<&'a Resp> {
    let Resp::Map(pairs) = response else {
        return None;
    };
    pairs.iter().find_map(|(entry_key, value)| match entry_key {
        Resp::Bulk(Some(entry)) if entry == key.as_bytes() => Some(value),
        _ => None,
    })
}

#[tokio::test]
async fn replica_redis_rejects_writes_without_advancing_the_sequence() {
    let server = spawn_with_role("compat_replica_redis_writes", ReplicationRole::Replica).await;
    let mut client = RedisClient::connect(&server.redis_addr).await;
    let before = server.server.db().newest_sequence();

    for arguments in [
        vec![&b"SET"[..], b"key", b"value"],
        vec![&b"SETEX"[..], b"key", b"10", b"value"],
        vec![&b"SETNX"[..], b"key", b"value"],
        vec![&b"MSET"[..], b"a", b"1", b"b", b"2"],
        vec![&b"DEL"[..], b"key"],
        vec![&b"INCR"[..], b"counter"],
        vec![&b"APPEND"[..], b"key", b"suffix"],
        vec![&b"EXPIRE"[..], b"key", b"10"],
        vec![&b"PERSIST"[..], b"key"],
        vec![&b"FLUSHDB"[..]],
        vec![&b"FLUSHALL"[..]],
    ] {
        match client.command(&arguments).await {
            Resp::Error(message) => assert!(
                message.starts_with("READONLY"),
                "unexpected error for {arguments:?}: {message}"
            ),
            other => panic!("replica accepted {arguments:?}: {other:?}"),
        }
    }

    assert_eq!(server.server.db().newest_sequence(), before);
    let default = scope_handle(server.server.db(), "default");
    assert_eq!(
        server
            .server
            .db()
            .get(&default, "key")
            .expect("read default"),
        None
    );
}

#[tokio::test]
async fn replica_redis_reads_do_not_create_scopes() {
    let server = spawn_with_role("compat_replica_redis_reads", ReplicationRole::Replica).await;
    let mut client = RedisClient::connect(&server.redis_addr).await;
    let before = server.server.db().newest_sequence();

    assert!(matches!(
        client.command(&[b"SELECT", b"1"]).await,
        Resp::Error(_)
    ));
    assert!(!has_scope(server.server.db(), "redis_db_1"));
    assert_eq!(server.server.db().newest_sequence(), before);

    assert_eq!(
        client.command(&[b"GET", b"missing"]).await,
        Resp::Bulk(None)
    );
}

#[tokio::test]
async fn replica_redis_serves_existing_data_and_reports_replica_role() {
    let server = spawn_with_role("compat_replica_redis_read_ok", ReplicationRole::Replica).await;
    let db = server.server.db();
    let scope = scope_handle(db, "default");
    db.put(&scope, "present", "value").expect("seed value");

    let mut client = RedisClient::connect(&server.redis_addr).await;
    assert_eq!(
        client.command(&[b"GET", b"present"]).await,
        Resp::Bulk(Some(b"value".to_vec()))
    );
    assert_eq!(
        client.command(&[b"GET", b"missing"]).await,
        Resp::Bulk(None)
    );

    let hello = client.command(&[b"HELLO", b"3"]).await;
    assert_eq!(
        map_value(&hello, "role"),
        Some(&Resp::Bulk(Some(b"replica".to_vec())))
    );
}

#[tokio::test]
async fn replica_memcached_rejects_writes_and_serves_reads() {
    let server = spawn_with_role("compat_replica_memcached", ReplicationRole::Replica).await;
    let mut client = MemcachedClient::connect(&server.memcached_addr).await;
    let before = server.server.db().newest_sequence();

    for request in [
        &b"set key 0 0 5\r\nvalue\r\n"[..],
        &b"add key 0 0 5\r\nvalue\r\n"[..],
        &b"delete key\r\n"[..],
        &b"incr counter 1\r\n"[..],
        &b"decr counter 1\r\n"[..],
        &b"touch key 10\r\n"[..],
        &b"flush_all\r\n"[..],
    ] {
        assert_eq!(
            client.send(request, b"\r\n").await,
            b"SERVER_ERROR replica is read-only\r\n",
            "unexpected reply for {:?}",
            String::from_utf8_lossy(request)
        );
    }

    assert_eq!(server.server.db().newest_sequence(), before);
    assert!(!has_scope(server.server.db(), "memcached"));

    create_scope(server.server.db(), "memcached");
    assert_eq!(
        client.send(b"get missing\r\n", b"END\r\n").await,
        b"END\r\n"
    );
}

#[tokio::test]
async fn replica_etcd_rejects_writes_and_serves_reads() {
    let server = spawn_with_role("compat_replica_etcd", ReplicationRole::Replica).await;
    let before = server.server.db().newest_sequence();
    let key = base64_key(b"alpha");

    let (status, reply) = http_post(
        &server,
        "/v3/kv/put",
        &json!({"key": key, "value": key}).to_string(),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(reply["code"], 9);

    let (status, reply) = http_post(
        &server,
        "/v3/kv/deleterange",
        &json!({"key": key}).to_string(),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(reply["code"], 9);

    assert_eq!(server.server.db().newest_sequence(), before);
    assert!(!has_scope(server.server.db(), "etcd"));

    create_scope(server.server.db(), "etcd");
    let (status, reply) =
        http_post(&server, "/v3/kv/range", &json!({"key": key}).to_string()).await;
    assert_eq!(status, 200);
    assert_eq!(reply["count"], "0");
}

fn base64_key(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
