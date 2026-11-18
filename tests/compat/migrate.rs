use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use storkv_cli::{CliRunner, MigrateTarget};
use storkv_core::{Db, DbOptions};

use crate::common::TempDb;
use crate::support::{RedisClient, spawn};

static FILE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn temp_jsonl(label: &str) -> PathBuf {
    let count = FILE_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "storkv_compat_migrate_{label}_{}_{}.jsonl",
        std::process::id(),
        count
    ))
}

fn record(key: &[u8], value: &[u8], expires_at_ms: u64) -> String {
    format!(
        "{{\"key_b64\":\"{}\",\"value_b64\":\"{}\",\"expires_at_ms\":{expires_at_ms}}}",
        STANDARD.encode(key),
        STANDARD.encode(value)
    )
}

#[tokio::test]
async fn jsonl_roundtrip_through_daemon() {
    let server = spawn("compat_migrate_daemon").await;
    let db = server.server.db();
    let source = db
        .create_scope("migrate_source", storkv_core::ScopeOptions::default())
        .expect("create source scope");
    db.put(&source, b"alpha", b"one").expect("put alpha");
    db.put_with_ttl(
        &source,
        b"beta",
        b"two",
        std::time::Duration::from_secs(600),
    )
    .expect("put beta");

    let file = temp_jsonl("roundtrip");
    let exported = CliRunner::migrate_export(&server.tcp_uri, "migrate_source", &file)
        .await
        .expect("export jsonl");
    assert_eq!(exported, 2);

    let imported = CliRunner::migrate_import(
        MigrateTarget::Daemon(&server.tcp_uri),
        "migrate_target",
        &file,
    )
    .await
    .expect("import jsonl");
    assert_eq!(imported, 2);

    let target_id = db
        .inner()
        .scope_names
        .read()
        .get("migrate_target")
        .copied()
        .expect("target scope registered");
    let target = storkv_core::ScopeHandle {
        scope_id: target_id,
        name: "migrate_target".into(),
    };
    assert_eq!(
        db.get(&target, b"alpha").expect("get alpha"),
        Some(bytes("one"))
    );
    assert_eq!(
        db.get(&target, b"beta").expect("get beta"),
        Some(bytes("two"))
    );
    let meta = db
        .key_meta(&target, b"beta")
        .expect("beta meta")
        .expect("beta present");
    assert!(meta.expires_at_ms > 0);

    let _ = std::fs::remove_file(&file);
}

#[tokio::test]
async fn jsonl_import_into_offline_data_dir() {
    let mut temp = TempDb::new("compat_migrate_offline");
    let directory = temp.path.clone();
    temp.close();

    let file = temp_jsonl("offline");
    let contents = format!(
        "{}\n{}\n",
        record(b"offline_key", b"offline_value", 0),
        record(
            b"ttl_key",
            b"ttl_value",
            storkv_core::TimestampMs::now().get() + 600_000
        ),
    );
    std::fs::write(&file, contents).expect("write jsonl");

    let imported = CliRunner::migrate_import(MigrateTarget::DataDir(&directory), "offline", &file)
        .await
        .expect("offline import");
    assert_eq!(imported, 2);

    let db = Db::open(&directory, DbOptions::default()).expect("reopen data dir");
    let scope_id = db
        .inner()
        .scope_names
        .read()
        .get("offline")
        .copied()
        .expect("offline scope");
    let scope = storkv_core::ScopeHandle {
        scope_id,
        name: "offline".into(),
    };
    assert_eq!(
        db.get(&scope, b"offline_key").expect("get key"),
        Some(bytes("offline_value"))
    );
    let meta = db
        .key_meta(&scope, b"ttl_key")
        .expect("ttl meta")
        .expect("ttl present");
    assert!(meta.expires_at_ms > 0);

    drop(db);
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(directory);
}

#[tokio::test]
async fn migrate_from_redis_preserves_values_and_ttl() {
    let server = spawn("compat_migrate_redis").await;
    let mut redis = RedisClient::connect(&server.redis_addr).await;
    redis.command(&[b"SET", b"plain", b"value"]).await;
    redis
        .command(&[b"SET", b"expiring", b"soon", b"EX", b"600"])
        .await;

    let imported = CliRunner::migrate_from_redis(
        &server.redis_addr,
        MigrateTarget::Daemon(&server.tcp_uri),
        "redis_import",
    )
    .await
    .expect("migrate from redis");
    assert_eq!(imported, 2);

    let db = server.server.db();
    let scope_id = db
        .inner()
        .scope_names
        .read()
        .get("redis_import")
        .copied()
        .expect("redis_import scope");
    let scope = storkv_core::ScopeHandle {
        scope_id,
        name: "redis_import".into(),
    };
    assert_eq!(
        db.get(&scope, b"plain").expect("get plain"),
        Some(bytes("value"))
    );
    assert_eq!(
        db.get(&scope, b"expiring").expect("get expiring"),
        Some(bytes("soon"))
    );
    assert!(
        db.key_ttl_remaining_ms(&scope, b"expiring")
            .expect("ttl")
            .unwrap_or(0)
            > 0
    );
}

fn bytes(value: &str) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(value.as_bytes())
}
