use std::sync::Arc;

use storkv_cli::{CliRunner, MigrateTarget};
use storkv_server::Server;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn temp_jsonl(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "storkv_cli_migrate_{label}_{}.jsonl",
        std::process::id()
    ))
}

async fn redis_set(address: &str, key: &str, value: &str) {
    let mut stream = TcpStream::connect(address).await.expect("redis connect");
    let frame = format!(
        "*3\r\n$3\r\nSET\r\n${}\r\n{}\r\n${}\r\n{}\r\n",
        key.len(),
        key,
        value.len(),
        value
    );
    stream
        .write_all(frame.as_bytes())
        .await
        .expect("redis write");
    let mut reply = [0u8; 5];
    stream.read_exact(&mut reply).await.expect("redis read");
    assert_eq!(&reply, b"+OK\r\n");
}

#[tokio::test]
async fn cli_migrate_jsonl_roundtrip_between_scopes() {
    let (server, addr, _temp_db) = crate::spawn_test_server("cli_migrate_jsonl").await;
    let uri = format!("storkv://{addr}");
    let source = server
        .db()
        .create_scope("cli_source", storkv_core::ScopeOptions::default())
        .unwrap();
    server.db().put(&source, b"k1", b"v1").unwrap();
    server.db().put(&source, b"k2", b"v2").unwrap();

    let file = temp_jsonl("roundtrip");
    let exported = CliRunner::migrate_export(&uri, "cli_source", &file)
        .await
        .unwrap();
    assert_eq!(exported, 2);

    let imported = CliRunner::migrate_import(MigrateTarget::Daemon(&uri), "cli_target", &file)
        .await
        .unwrap();
    assert_eq!(imported, 2);

    let scope_id = server
        .db()
        .inner()
        .scope_names
        .read()
        .get("cli_target")
        .copied()
        .unwrap();
    let target = storkv_core::ScopeHandle {
        scope_id,
        name: "cli_target".into(),
    };
    assert_eq!(
        server.db().get(&target, b"k1").unwrap().as_deref(),
        Some(b"v1".as_slice())
    );
    assert_eq!(
        server.db().get(&target, b"k2").unwrap().as_deref(),
        Some(b"v2".as_slice())
    );

    let _ = std::fs::remove_file(&file);
    let _ = server.shutdown_handle().send(());
}

#[tokio::test]
async fn cli_migrate_jsonl_preserves_ttl_metadata() {
    let (server, addr, _temp_db) = crate::spawn_test_server("cli_migrate_ttl").await;
    let uri = format!("storkv://{addr}");
    let source = server
        .db()
        .create_scope("ttl_source", storkv_core::ScopeOptions::default())
        .unwrap();
    server
        .db()
        .put_with_ttl(
            &source,
            b"ttl-key",
            b"ttl-value",
            std::time::Duration::from_secs(120),
        )
        .unwrap();

    let file = temp_jsonl("ttl");
    assert_eq!(
        CliRunner::migrate_export(&uri, "ttl_source", &file)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        CliRunner::migrate_import(MigrateTarget::Daemon(&uri), "ttl_target", &file)
            .await
            .unwrap(),
        1
    );

    let scope_id = server
        .db()
        .inner()
        .scope_names
        .read()
        .get("ttl_target")
        .copied()
        .unwrap();
    let target = storkv_core::ScopeHandle {
        scope_id,
        name: "ttl_target".into(),
    };
    let meta = server
        .db()
        .key_meta(&target, b"ttl-key")
        .unwrap()
        .expect("migrated key keeps metadata");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    assert!(
        meta.expires_at_ms > now_ms + 100_000,
        "TTL must survive export/import: {meta:?}"
    );

    let _ = std::fs::remove_file(&file);
    let _ = server.shutdown_handle().send(());
}

#[tokio::test]
async fn cli_migrate_from_redis_imports_keys() {
    let (server, addr, _temp_db) = crate::spawn_test_server("cli_migrate_redis").await;
    let redis_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redis_addr = redis_listener.local_addr().unwrap().to_string();
    let redis_server: Arc<Server> = server.clone();
    tokio::spawn(async move {
        let _ = storkv_server::compat::serve_redis_tcp(redis_server, redis_listener).await;
    });

    redis_set(&redis_addr, "alpha", "one").await;
    redis_set(&redis_addr, "beta", "two").await;

    let uri = format!("storkv://{addr}");
    let imported =
        CliRunner::migrate_from_redis(&redis_addr, MigrateTarget::Daemon(&uri), "cli_redis")
            .await
            .unwrap();
    assert_eq!(imported, 2);

    let scope_id = server
        .db()
        .inner()
        .scope_names
        .read()
        .get("cli_redis")
        .copied()
        .unwrap();
    let scope = storkv_core::ScopeHandle {
        scope_id,
        name: "cli_redis".into(),
    };
    let rows = server
        .db()
        .scan(&scope, None::<&[u8]>, None::<&[u8]>)
        .unwrap();
    assert_eq!(rows.len(), 2);

    let _ = server.shutdown_handle().send(());
}
