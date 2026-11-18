use std::fs;

use storkv_core::backup::{self, BackupKind, BackupManifest};

use crate::support::{backup_root, generation_directories};

#[test]
fn local_sink_publishes_generation_atomically() {
    let mut temp = crate::common::TempDb::new("local_sink");
    let db = temp.db();
    let scope = db
        .create_scope("sink_scope", storkv_core::ScopeOptions::default())
        .unwrap();
    for index in 0..16 {
        db.put(
            &scope,
            format!("k{index}").as_bytes(),
            format!("v{index}").as_bytes(),
        )
        .unwrap();
    }

    let root = backup_root(&temp);
    db.start_auto_backup(backup::BackupConfig::local(root.clone()))
        .unwrap();
    let report = db.trigger_backup(true).unwrap();
    db.stop_auto_backup().unwrap();

    let generations = generation_directories(&root);
    assert!(
        !generations.is_empty(),
        "starting the worker or an explicit trigger must publish a generation"
    );
    let published = generations
        .iter()
        .find(|path| path.file_name().unwrap().to_string_lossy() == report.generation_id)
        .expect("explicitly triggered generation must be published");

    // The unpublished staging area must not retain the generation.
    assert!(!root.join(".tmp").join(&report.generation_id).exists());

    let manifest = BackupManifest::read_from_dir(published).unwrap();
    assert_eq!(manifest.format, "STORKBACKUP");
    assert_eq!(manifest.kind, BackupKind::Full);
    assert_eq!(manifest.max_seq, report.sequence);
    assert!(manifest.total_bytes > 0);
    assert!(!manifest.files.is_empty());

    let sidecar = published.join("BACKUP_MANIFEST.json.crc32c");
    assert!(sidecar.exists(), "manifest sidecar must exist");
    assert!(fs::read_to_string(sidecar).unwrap().starts_with("0x"));

    let catalog = backup::load_catalog(&root).unwrap();
    assert!(
        catalog
            .iter()
            .any(|manifest| manifest.generation_id == report.generation_id)
    );

    temp.close();
}

#[test]
fn local_sink_retention_prunes_old_generations() {
    let mut temp = crate::common::TempDb::new("local_retention");
    let db = temp.db();
    let scope = db
        .create_scope("retain_scope", storkv_core::ScopeOptions::default())
        .unwrap();
    let root = backup_root(&temp);
    let config =
        backup::BackupConfig::local(root.clone()).with_retention(backup::BackupRetention {
            max_full_backups: 2,
            ..backup::BackupRetention::default()
        });
    db.start_auto_backup(config).unwrap();

    for index in 0..4 {
        db.put(
            &scope,
            format!("k{index}").as_bytes(),
            format!("v{index}").as_bytes(),
        )
        .unwrap();
        db.trigger_backup(true).unwrap();
    }
    db.stop_auto_backup().unwrap();

    let generations = generation_directories(&root);
    assert_eq!(generations.len(), 2, "retention must keep two generations");
    let catalog = backup::load_catalog(&root).unwrap();
    assert_eq!(catalog.len(), 2);
    assert!(
        catalog[0].max_seq < catalog[1].max_seq,
        "catalog must be ordered oldest to newest"
    );

    temp.close();
}
