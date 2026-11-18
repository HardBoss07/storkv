use std::fs;

use storkv_core::backup::{self, RestoreReport};
use storkv_core::wal::WalBatchFrame;
use storkv_core::{Db, DbOptions, ScopeOptions, StorKvError};

use crate::support::{backup_root, put_op, write_wal_segment};

fn prepare(prefix: &str) -> (crate::common::TempDb, std::path::PathBuf, u32, u64) {
    let temp = crate::common::TempDb::new(prefix);
    let db = temp.db();
    let scope = db.create_scope("pages", ScopeOptions::default()).unwrap();
    for index in 0..8 {
        db.put(
            &scope,
            format!("base-{index}").as_bytes(),
            format!("base-value-{index}").as_bytes(),
        )
        .unwrap();
    }
    let root = backup_root(&temp);
    db.start_auto_backup(backup::BackupConfig::local(root.clone()))
        .unwrap();
    db.trigger_backup(true).unwrap();
    db.stop_auto_backup().unwrap();

    let base_seq = backup::load_catalog(&root)
        .unwrap()
        .first()
        .unwrap()
        .max_seq;
    (temp, root, scope.scope_id, base_seq)
}

#[test]
fn restore_latest_replays_every_archived_operation() {
    let (mut temp, root, scope_id, base_seq) = prepare("restore_latest");
    temp.close();

    write_wal_segment(
        &root,
        0xffff,
        WalBatchFrame::new(
            base_seq + 1,
            vec![
                put_op(scope_id, "pitr-a", "a"),
                put_op(scope_id, "pitr-b", "b"),
                put_op(scope_id, "pitr-c", "c"),
            ],
        ),
    );

    let target = temp.path.join("restored");
    let report: RestoreReport = Db::restore_from_backup(&root, &target, None).unwrap();
    assert_eq!(report.restored_seq, base_seq + 3);
    assert_eq!(report.wal_segments_replayed, 1);
    assert!(report.bytes_restored > 0);

    let db = Db::open(&target, DbOptions::default()).unwrap();
    let scope = storkv_core::ScopeHandle {
        scope_id,
        name: "pages".into(),
    };
    assert_eq!(
        db.get(&scope, b"base-0").unwrap().as_deref(),
        Some(b"base-value-0".as_slice())
    );
    assert_eq!(
        db.get(&scope, b"pitr-c").unwrap().as_deref(),
        Some(b"c".as_slice())
    );
}

#[test]
fn restore_mid_batch_truncates_operations_above_target() {
    let (mut temp, root, scope_id, base_seq) = prepare("restore_mid_batch");
    temp.close();

    write_wal_segment(
        &root,
        0xffff,
        WalBatchFrame::new(
            base_seq + 1,
            vec![
                put_op(scope_id, "pitr-a", "a"),
                put_op(scope_id, "pitr-b", "b"),
                put_op(scope_id, "pitr-c", "c"),
            ],
        ),
    );

    let target = temp.path.join("restored-mid");
    let report = Db::restore_from_backup(&root, &target, Some(base_seq + 2)).unwrap();
    assert_eq!(report.restored_seq, base_seq + 2);

    let db = Db::open(&target, DbOptions::default()).unwrap();
    let scope = storkv_core::ScopeHandle {
        scope_id,
        name: "pages".into(),
    };
    assert!(db.get(&scope, b"pitr-a").unwrap().is_some());
    assert!(db.get(&scope, b"pitr-b").unwrap().is_some());
    assert!(
        db.get(&scope, b"pitr-c").unwrap().is_none(),
        "operations above the target sequence must not be replayed"
    );
}

#[test]
fn restore_rejects_target_above_newest_archived_sequence() {
    let (mut temp, root, _scope_id, base_seq) = prepare("restore_above");
    temp.close();

    let target = temp.path.join("restored-above");
    let error = Db::restore_from_backup(&root, &target, Some(base_seq + 10)).unwrap_err();
    match error {
        StorKvError::TargetSequenceUnavailable {
            requested, newest, ..
        } => {
            assert_eq!(requested, base_seq + 10);
            assert_eq!(newest, base_seq);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn restore_rejects_non_empty_target() {
    let (mut temp, root, _scope_id, _base_seq) = prepare("restore_nonempty");
    temp.close();

    let target = temp.path.join("occupied");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("keep.txt"), b"user data").unwrap();

    let error = Db::restore_from_backup(&root, &target, None).unwrap_err();
    assert!(matches!(error, StorKvError::RestoreTargetNotEmpty(_)));
    assert_eq!(fs::read(target.join("keep.txt")).unwrap(), b"user data");
}
