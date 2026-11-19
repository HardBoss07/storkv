use storkv_cli::CliRunner;
use storkv_core::backup::{self, BackupConfig};

#[tokio::test]
async fn cli_backup_status_and_verify_use_wire_opcodes() {
    let (server, addr, temp_db) = crate::spawn_test_server("cli_backup_status").await;
    let root = temp_db.path.join("auto_backups");
    server
        .db()
        .start_auto_backup(BackupConfig::local(root.clone()))
        .unwrap();
    server.db().trigger_backup(true).unwrap();

    let status = CliRunner::backup_status(&addr).await.unwrap();
    assert!(status.enabled);
    assert_eq!(status.backup_root.as_deref(), root.to_str());
    assert!(status.backups_completed_total >= 1);
    assert_eq!(status.sink, "local");

    let report = CliRunner::backup_verify(&addr, None).await.unwrap();
    assert!(report.is_ok(), "verification errors: {:?}", report.errors);
    assert!(report.generations_checked >= 1);

    server.db().stop_auto_backup().unwrap();
    server.shutdown_handle().send(()).unwrap();
}

#[tokio::test]
async fn cli_restore_latest_and_target_sequence() {
    let (server, _addr, temp_db) = crate::spawn_test_server("cli_restore").await;
    let root = temp_db.path.join("auto_backups");
    server
        .db()
        .start_auto_backup(BackupConfig::local(root.clone()))
        .unwrap();
    let scope = storkv_core::ScopeHandle {
        scope_id: 0,
        name: "default".into(),
    };
    server
        .db()
        .put(&scope, b"restored-key", b"restored-value")
        .unwrap();
    let report = server.db().trigger_backup(true).unwrap();
    server.db().stop_auto_backup().unwrap();

    let out = temp_db.path.join("restored");
    let restored = CliRunner::restore(&root, &out, None).unwrap();
    assert_eq!(restored.restored_seq, report.sequence);

    let db = storkv_core::Db::open(&out, storkv_core::DbOptions::default()).unwrap();
    assert_eq!(
        db.get(&scope, b"restored-key").unwrap().as_deref(),
        Some(b"restored-value".as_slice())
    );
    drop(db);

    let occupied = temp_db.path.join("occupied");
    std::fs::create_dir_all(&occupied).unwrap();
    std::fs::write(occupied.join("keep"), b"x").unwrap();
    let error = CliRunner::restore(&root, &occupied, None).unwrap_err();
    assert!(matches!(
        error,
        storkv_core::StorkvError::RestoreTargetNotEmpty(_)
    ));

    let _ = server.shutdown_handle().send(());
}

#[test]
fn verify_empty_backup_directory_is_a_noop() {
    let temp = std::env::temp_dir().join(format!("storkv_cli_no_backup_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    let report = backup::verify_backup(&temp, None).unwrap();
    assert!(report.is_ok());
    assert_eq!(report.generations_checked, 0);
    let _ = std::fs::remove_dir_all(&temp);
}
