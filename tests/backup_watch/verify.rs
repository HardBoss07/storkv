use std::fs;
use std::path::Path;

use storkv_core::backup;

use crate::support::{backup_root, generation_directories};

#[test]
fn verify_backup_accepts_clean_generations() {
    let mut temp = crate::common::TempDb::new("verify_clean");
    let db = temp.db();
    let scope = db
        .create_scope("verify_scope", storkv_core::ScopeOptions::default())
        .unwrap();
    for index in 0..32 {
        db.put(
            &scope,
            format!("k{index:02}").as_bytes(),
            format!("v{index:02}").as_bytes(),
        )
        .unwrap();
    }
    let root = backup_root(&temp);
    db.start_auto_backup(backup::BackupConfig::local(root.clone()))
        .unwrap();
    db.trigger_backup(true).unwrap();
    db.stop_auto_backup().unwrap();

    let report = backup::verify_backup(&root, None).unwrap();
    assert!(report.is_ok(), "clean backup reported {:?}", report.errors);
    assert!(report.generations_checked >= 1);
    assert!(report.files_checked > 0);
    assert!(report.bytes_checked > 0);

    temp.close();
}

#[test]
fn verify_backup_rejects_flipped_sstable_byte() {
    let mut temp = crate::common::TempDb::new("verify_flipped");
    let db = temp.db();
    let scope = db
        .create_scope("flip_scope", storkv_core::ScopeOptions::default())
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
    db.trigger_backup(true).unwrap();
    db.stop_auto_backup().unwrap();
    temp.close();

    let generation = generation_directories(&root).remove(0);
    let sstable = first_file_with_extension(&generation, "sst");
    let mut bytes = fs::read(&sstable).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    fs::write(&sstable, &bytes).unwrap();

    let report = backup::verify_backup(&root, None).unwrap();
    assert!(!report.is_ok(), "flipped byte must be detected");
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.detail.to_ascii_lowercase().contains("crc")),
        "expected a CRC mismatch, got {:?}",
        report.errors
    );
}

#[test]
fn verify_backup_rejects_truncated_manifest_and_sidecar() {
    let mut temp = crate::common::TempDb::new("verify_manifest");
    let db = temp.db();
    let scope = db
        .create_scope("manifest_scope", storkv_core::ScopeOptions::default())
        .unwrap();
    db.put(&scope, b"k", b"v").unwrap();
    let root = backup_root(&temp);
    db.start_auto_backup(backup::BackupConfig::local(root.clone()))
        .unwrap();
    db.trigger_backup(true).unwrap();
    db.stop_auto_backup().unwrap();
    temp.close();

    let generation = generation_directories(&root).remove(0);
    let sidecar = generation.join("BACKUP_MANIFEST.json.crc32c");
    fs::write(&sidecar, b"torn").unwrap();

    let error = backup::verify_backup(&root, None).unwrap_err();
    assert!(
        matches!(error, storkv_core::StorkvError::BackupCorruption { .. }),
        "torn sidecar must be rejected, got {error}"
    );

    fs::write(&sidecar, b"0x00000000\n").unwrap();
    let error = backup::verify_backup(&root, None).unwrap_err();
    assert!(
        matches!(error, storkv_core::StorkvError::BackupCorruption { .. }),
        "wrong sidecar checksum must be rejected, got {error}"
    );
}

#[test]
fn verify_backup_rejects_target_outside_catalog() {
    let mut temp = crate::common::TempDb::new("verify_target");
    let db = temp.db();
    let scope = db
        .create_scope("target_scope", storkv_core::ScopeOptions::default())
        .unwrap();
    db.put(&scope, b"k", b"v").unwrap();
    let root = backup_root(&temp);
    db.start_auto_backup(backup::BackupConfig::local(root.clone()))
        .unwrap();
    let report = db.trigger_backup(true).unwrap();
    db.stop_auto_backup().unwrap();
    temp.close();

    let error = backup::verify_backup(&root, Some(report.sequence + 1000)).unwrap_err();
    assert!(matches!(
        error,
        storkv_core::StorkvError::TargetSequenceUnavailable { .. }
    ));
}

fn first_file_with_extension(directory: &Path, extension: &str) -> std::path::PathBuf {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if let Some(found) = try_first_file(&path, extension) {
                return found;
            }
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) == Some(extension) {
            return path;
        }
    }
    panic!("no {extension} file under {}", directory.display());
}

fn try_first_file(directory: &Path, extension: &str) -> Option<std::path::PathBuf> {
    for entry in fs::read_dir(directory).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(found) = try_first_file(&path, extension) {
                return Some(found);
            }
        } else if path.extension().and_then(|value| value.to_str()) == Some(extension) {
            return Some(path);
        }
    }
    None
}
