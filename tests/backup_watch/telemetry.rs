use std::sync::atomic::Ordering;

use storkv_core::ScopeOptions;

#[test]
fn test_lock_free_telemetry_counters() {
    let mut temp = crate::common::TempDb::new("telemetry_test");
    let db = temp.db();

    let scope = db
        .create_scope("telem_scope", ScopeOptions::default())
        .unwrap();

    let writes_before = db.telemetry().writes_total.load(Ordering::Relaxed);
    let reads_before = db.telemetry().reads_total.load(Ordering::Relaxed);

    db.put(&scope, b"t_key", b"t_val").unwrap();
    let _ = db.get(&scope, b"t_key").unwrap();

    let writes_after = db.telemetry().writes_total.load(Ordering::Relaxed);
    let reads_after = db.telemetry().reads_total.load(Ordering::Relaxed);

    assert_eq!(writes_after, writes_before + 1);
    assert_eq!(reads_after, reads_before + 1);

    temp.close();
}

#[test]
fn test_backup_telemetry_counters_update() {
    let mut temp = crate::common::TempDb::new("telemetry_backup");
    let db = temp.db();
    let scope = db
        .create_scope("backup_telem", ScopeOptions::default())
        .unwrap();
    db.put(&scope, b"key", b"val").unwrap();

    let backup_root = temp.path.join("backups");
    db.start_auto_backup(storkv_core::backup::BackupConfig::local(
        backup_root.clone(),
    ))
    .unwrap();

    let before = db
        .telemetry()
        .backups_completed_total
        .load(Ordering::Relaxed);
    let report = db.trigger_backup(true).unwrap();
    let after = db
        .telemetry()
        .backups_completed_total
        .load(Ordering::Relaxed);

    assert_eq!(after, before + 1);
    assert_eq!(
        report.sequence,
        db.inner().last_visible_seq.load(Ordering::Acquire)
    );
    assert!(db.telemetry().last_backup_seq.load(Ordering::Relaxed) >= report.sequence);
    assert!(
        db.telemetry()
            .last_backup_timestamp_ms
            .load(Ordering::Relaxed)
            > 0
    );

    db.stop_auto_backup().unwrap();
    temp.close();
}
