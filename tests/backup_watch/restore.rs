use std::fs;

use storkv_core::backup::{self, BackupKind, BackupManifest, RestoreReport};
use storkv_core::wal::WalBatchFrame;
use storkv_core::{Db, DbOptions, ScopeOptions, StorkvError};

#[cfg(unix)]
use crate::support::generation_directories;
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
        StorkvError::TargetSequenceUnavailable {
            requested, newest, ..
        } => {
            assert_eq!(requested, base_seq + 10);
            assert_eq!(newest, base_seq);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn manifest_rejects_path_traversal_generation_id() {
    for generation_id in [
        "",
        ".",
        "..",
        "../../outside",
        "nested/path",
        "nested\\path",
    ] {
        let manifest =
            BackupManifest::new(generation_id.to_string(), BackupKind::Full, 0, 1, 1, "none");
        let error = manifest
            .validate()
            .expect_err("unsafe generation identifier must fail");
        assert!(
            matches!(error, StorkvError::BackupCorruption { .. }),
            "generation id {generation_id:?} returned {error:?}"
        );
    }

    let mut manifest = BackupManifest::new("safe".into(), BackupKind::Full, 0, 1, 1, "none");
    for path in [
        "../escape.sst",
        "..\\escape.sst",
        "/absolute.sst",
        "\\absolute.sst",
        "C:/windows.sst",
    ] {
        manifest.files = vec![storkv_core::backup::BackupFileEntry {
            size_bytes: 1,
            min_seq: None,
            max_seq: None,
            path: path.to_string(),
            kind: storkv_core::backup::BackupFileKind::Sst,
            crc32c: "0x00000000".into(),
        }];
        let error = manifest.validate().expect_err("unsafe file path must fail");
        assert!(
            matches!(error, StorkvError::BackupCorruption { .. }),
            "file path {path:?} returned {error:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn restore_rejects_symlink_in_backup_tree() {
    use std::os::unix::fs::symlink;

    let (mut temp, root, _scope_id, _base_seq) = prepare("restore_symlink");
    temp.close();

    let generation = generation_directories(&root).remove(0);
    symlink("/etc/passwd", generation.join("escape")).expect("create symlink");

    let target = temp.path.join("restored-symlink");
    let error = Db::restore_from_backup(&root, &target, None)
        .expect_err("a symlink must block the restore");
    assert!(
        matches!(error, StorkvError::BackupCorruption { .. }),
        "unexpected error: {error:?}"
    );
    assert!(
        !target.exists(),
        "a failed restore must not create a target"
    );
}

#[test]
fn restore_rejects_non_empty_target() {
    let (mut temp, root, _scope_id, _base_seq) = prepare("restore_nonempty");
    temp.close();

    let target = temp.path.join("occupied");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("keep.txt"), b"user data").unwrap();

    let error = Db::restore_from_backup(&root, &target, None).unwrap_err();
    assert!(matches!(error, StorkvError::RestoreTargetNotEmpty(_)));
    assert_eq!(fs::read(target.join("keep.txt")).unwrap(), b"user data");
}
