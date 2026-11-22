use std::fs;

use storkv_core::backup::{BackupCompression, BackupConfig, CommandPipeSinkConfig};

use crate::support::{backup_root, generation_directories};

#[test]
fn crash_before_publish_keeps_previous_generation_restorable() {
    let temp = crate::common::TempDb::new("chaos_staging");
    let db = temp.db();
    let scope = db
        .create_scope("chaos_scope", storkv_core::ScopeOptions::default())
        .unwrap();
    db.put(&scope, b"stable", b"value").unwrap();
    let root = backup_root(&temp);
    db.start_auto_backup(BackupConfig::local(root.clone()))
        .unwrap();
    let first = db.trigger_backup(true).unwrap();
    db.stop_auto_backup().unwrap();

    // Simulate a crash while staging the next generation.
    let staged = root.join(".tmp").join("full-crashed-0000000000000099");
    fs::create_dir_all(staged.join("scopes")).unwrap();
    fs::write(staged.join("CURRENT"), b"MANIFEST-broken").unwrap();

    let catalog = storkv_core::backup::load_catalog(&root).unwrap();
    assert!(!catalog.is_empty(), "published generations must survive");
    assert!(
        catalog
            .iter()
            .all(|manifest| manifest.generation_id != "full-crashed-0000000000000099"),
        "staged leftovers must not be cataloged"
    );
    assert!(
        catalog
            .iter()
            .any(|manifest| manifest.generation_id == first.generation_id)
    );

    let target = temp.path.join("restored");
    let report = storkv_core::Db::restore_from_backup(&root, &target, None).unwrap();
    assert_eq!(report.generation_id, catalog.last().unwrap().generation_id);
    assert_eq!(
        storkv_core::Db::open(&target, storkv_core::DbOptions::default())
            .unwrap()
            .get(
                &storkv_core::ScopeHandle {
                    scope_id: scope.scope_id,
                    name: "chaos_scope".into()
                },
                b"stable"
            )
            .unwrap()
            .as_deref(),
        Some(b"value".as_slice())
    );
}

#[test]
fn failing_sink_reports_failure_and_preserves_catalog() {
    let temp = crate::common::TempDb::new("chaos_failing_sink");
    let db = temp.db();
    let scope = db
        .create_scope("failing_scope", storkv_core::ScopeOptions::default())
        .unwrap();
    db.put(&scope, b"k", b"v").unwrap();
    let root = backup_root(&temp);

    db.start_auto_backup(BackupConfig::local(root.clone()))
        .unwrap();
    db.trigger_backup(true).unwrap();
    db.stop_auto_backup().unwrap();
    let generations_before = generation_directories(&root).len();

    let failing =
        BackupConfig::command(root.clone(), failing_command(BackupCompression::None)).unwrap();
    db.start_auto_backup(failing).unwrap();
    let error = db.trigger_backup(true).unwrap_err();
    assert!(
        matches!(error, storkv_core::StorkvError::BackupSink { .. }),
        "unexpected error: {error}"
    );
    assert!(
        db.telemetry()
            .backups_failed_total
            .load(std::sync::atomic::Ordering::Relaxed)
            >= 1
    );
    db.stop_auto_backup().unwrap();

    assert_eq!(
        generation_directories(&root).len(),
        generations_before,
        "a failed publish must not add a generation"
    );
}

#[cfg(windows)]
fn failing_command(compression: BackupCompression) -> CommandPipeSinkConfig {
    CommandPipeSinkConfig::new("cmd")
        .with_args(vec!["/C".into(), "exit 1".into()])
        .with_compression(compression)
}

#[cfg(unix)]
fn failing_command(compression: BackupCompression) -> CommandPipeSinkConfig {
    CommandPipeSinkConfig::new("sh")
        .with_args(vec!["-c".into(), "exit 1".into()])
        .with_compression(compression)
}
