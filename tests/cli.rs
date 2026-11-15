use std::io::Write;
use std::sync::Arc;
use storkv_cli::CliRunner;
use storkv_server::Server;
use tokio::net::TcpListener;

mod common;

async fn spawn_test_server(prefix: &str) -> (Arc<Server>, String, common::TempDb) {
    let mut temp_db = common::TempDb::new(prefix);
    let server = Arc::new(Server::with_db(Arc::new(temp_db.db.take().unwrap())));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let server_clone = server.clone();
    tokio::spawn(async move {
        server_clone
            .run_tcp_listener(listener, false)
            .await
            .unwrap();
    });

    (server, addr, temp_db)
}

#[tokio::test]
async fn test_cli_health_and_backup() {
    let (server, addr, temp_db) = spawn_test_server("cli_health_backup").await;

    // Healthcheck
    CliRunner::health(&addr).await.expect("Healthcheck failed");

    // Backup
    let backup_dir = temp_db.path.join("backups_out");
    CliRunner::backup(&addr, backup_dir.to_str().unwrap())
        .await
        .expect("Backup failed");

    assert!(backup_dir.exists());
    assert!(backup_dir.join("CURRENT").exists());

    server.shutdown_handle().send(()).unwrap();
}

#[tokio::test]
async fn test_cli_ycsb_smoke() {
    let (server, addr, _temp_db) = spawn_test_server("cli_ycsb").await;

    let summary_a = CliRunner::run_ycsb(&addr, "a", 20, 20)
        .await
        .expect("YCSB Workload A failed");
    assert_eq!(summary_a.workload, "a");
    assert_eq!(summary_a.total_ops, 40);

    let summary_b = CliRunner::run_ycsb(&addr, "b", 20, 20)
        .await
        .expect("YCSB Workload B failed");
    assert_eq!(summary_b.workload, "b");

    let summary_c = CliRunner::run_ycsb(&addr, "c", 20, 20)
        .await
        .expect("YCSB Workload C failed");
    assert_eq!(summary_c.workload, "c");

    server.shutdown_handle().send(()).unwrap();
}

#[test]
fn test_cli_doctor_healthy_and_corrupt() {
    let temp_db = common::TempDb::new("cli_doctor");
    let db = temp_db.db();

    let default_scope = storkv_core::ScopeHandle {
        scope_id: 0,
        name: "default".into(),
    };

    // Put enough entries to flush an SSTable
    for i in 0..100 {
        let k = format!("doctor_k_{:04}", i);
        let v = format!("doctor_v_{:04}", i);
        db.put(&default_scope, k.as_bytes(), v.as_bytes()).unwrap();
    }
    // Force a flush
    let scope_arc = db.inner().scopes.read().get(&0).cloned().unwrap();
    db.inner().flush_scope_memtable(&scope_arc).unwrap();

    // Run doctor on healthy database
    let report_clean = CliRunner::doctor(&temp_db.path);
    assert_eq!(report_clean.manifests_checked, 1);
    assert!(report_clean.sstables_checked >= 1);
    assert!(
        report_clean.errors.is_empty(),
        "Expected clean DB without errors, got: {:?}",
        report_clean.errors
    );

    // Corrupt one of the SSTable blocks
    let sst_dir = temp_db.path.join("scopes").join("00000000");
    let mut corrupted = false;
    if let Ok(entries) = std::fs::read_dir(&sst_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("sst") {
                let mut f = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&path)
                    .unwrap();
                f.write_all(b"CORRUPT_BYTES").unwrap();
                corrupted = true;
                break;
            }
        }
    }
    assert!(corrupted, "Failed to corrupt SSTable for test");

    // Run doctor on corrupted database
    let report_corrupted = CliRunner::doctor(&temp_db.path);
    assert!(
        !report_corrupted.errors.is_empty(),
        "Doctor should detect corrupted SSTable"
    );
}

// ---------------------------------------------------------------------------
// Hardening additions: doctor on corrupt MANIFEST/CURRENT, backup under live
// writes, and health checks against a dead server.
// ---------------------------------------------------------------------------

use storkv_core::{Db, DbOptions, ScopeHandle as CoreScopeHandle};

#[test]
fn test_doctor_reports_corrupt_manifest_and_missing_current() {
    let mut temp_db = common::TempDb::new("cli_doctor_manifest");
    {
        let db = temp_db.db();
        db.create_scope(
            "doctor_manifest_scope",
            storkv_core::ScopeOptions::default(),
        )
        .unwrap();
        db.create_snapshot("doctor_snap").unwrap();
    }
    temp_db.close();

    let current = std::fs::read_to_string(temp_db.path.join("CURRENT")).unwrap();
    let manifest_path = temp_db.path.join(current.trim());
    let mut data = std::fs::read(&manifest_path).unwrap();
    data[17] ^= 0x01;
    std::fs::write(&manifest_path, &data).unwrap();

    let report = CliRunner::doctor(&temp_db.path);
    assert_eq!(report.manifests_checked, 1);
    assert!(
        report
            .errors
            .iter()
            .any(|e| e.contains("manifest") || e.contains("Manifest")),
        "doctor must report manifest corruption: {:?}",
        report.errors
    );

    let empty = std::env::temp_dir().join(format!("storkv_doctor_empty_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&empty);
    std::fs::create_dir_all(&empty).unwrap();
    let report = CliRunner::doctor(&empty);
    assert!(report.errors.iter().any(|e| e.contains("CURRENT")));
    let _ = std::fs::remove_dir_all(&empty);
}

#[tokio::test]
async fn test_backup_under_live_writes_produces_openable_snapshot() {
    let (server, addr, temp_db) = spawn_test_server("cli_backup_live").await;
    let client = storkv_client::AsyncClient::connect(&addr).await.unwrap();
    let scope = storkv_client::ScopeHandle {
        scope_id: 0,
        name: "default".into(),
    };

    for index in 0..100 {
        client
            .put(
                &scope,
                format!("live:{:04}", index).as_bytes(),
                format!("value:{:04}", index).as_bytes(),
            )
            .await
            .unwrap();
    }

    let writer_client = client.clone();
    let writer_scope = scope.clone();
    let writer = tokio::spawn(async move {
        for index in 0..500 {
            writer_client
                .put(
                    &writer_scope,
                    format!("live:{:04}", index).as_bytes(),
                    format!("updated:{:04}", index).as_bytes(),
                )
                .await
                .unwrap();
        }
    });

    let backup_dir = temp_db.path.join("live_backup");
    CliRunner::backup(&addr, backup_dir.to_str().unwrap())
        .await
        .expect("backup under load");
    writer.await.unwrap();

    assert!(backup_dir.join("CURRENT").exists());
    assert!(backup_dir.join("wal").is_dir());

    let backup_db = Db::open(&backup_dir, DbOptions::default()).expect("backup must reopen");
    let handle = CoreScopeHandle {
        scope_id: 0,
        name: "default".into(),
    };
    let rows = backup_db.scan_prefix(&handle, b"live:").unwrap();
    assert!(!rows.is_empty(), "backup must contain committed rows");
    assert!(rows.len() <= 500);

    server.shutdown_handle().send(()).unwrap();
}

#[tokio::test]
async fn test_cli_health_against_dead_server_fails() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    drop(listener);

    let result = CliRunner::health(&addr).await;
    assert!(result.is_err(), "health against a dead server must fail");
}
