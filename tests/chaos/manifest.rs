use crate::common;
use std::fs;
use std::sync::Arc;
use storkv_core::manifest::ManifestManager;
use storkv_core::vfs::PosixFs;
use storkv_core::{Db, DbOptions, StorkvError};

#[test]
fn test_atomic_manifest_rolling_current() {
    let temp = std::env::temp_dir().join(format!("storkv_manifest_roll_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();

    let fs_posix = Arc::new(PosixFs);
    let manager = ManifestManager::open(fs_posix.clone(), &temp).unwrap();

    let current_path = temp.join("CURRENT");
    assert!(current_path.exists());
    let current_content = fs::read_to_string(&current_path).unwrap();
    assert!(current_content.starts_with("MANIFEST-"));

    drop(manager);
    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn test_corrupted_manifest_returns_corruption_not_panic() {
    let mut temp = common::TempDb::new("chaos_manifest_corrupt");
    {
        let db = temp.db();
        db.create_scope("manifest_scope", storkv_core::ScopeOptions::default())
            .unwrap();
        db.create_snapshot("manifest_snapshot").unwrap();
    }
    temp.close();

    let current = fs::read_to_string(temp.path.join("CURRENT")).unwrap();
    let manifest_name = current.trim();
    let manifest_path = temp.path.join(manifest_name);
    let mut data = fs::read(&manifest_path).unwrap();
    assert!(data.len() > 20, "manifest should contain at least one edit");
    data[17] ^= 0x01;
    fs::write(&manifest_path, &data).unwrap();

    let reopened = Db::open(&temp.path, DbOptions::default());
    assert!(
        matches!(reopened, Err(StorkvError::Corruption(_))),
        "corrupted manifest must return Corruption, got {:?}",
        reopened.err()
    );
}

#[test]
fn test_current_pointing_at_missing_or_invalid_manifest_fails_cleanly() {
    let missing =
        std::env::temp_dir().join(format!("storkv_current_missing_{}", std::process::id()));
    let _ = fs::remove_dir_all(&missing);
    fs::create_dir_all(&missing).unwrap();
    fs::write(missing.join("CURRENT"), b"MANIFEST-00000000000000ff\n").unwrap();
    let result = Db::open(&missing, DbOptions::default());
    assert!(result.is_err(), "missing manifest must fail to open");
    let _ = fs::remove_dir_all(&missing);

    let invalid =
        std::env::temp_dir().join(format!("storkv_current_invalid_{}", std::process::id()));
    let _ = fs::remove_dir_all(&invalid);
    fs::create_dir_all(&invalid).unwrap();
    fs::write(invalid.join("CURRENT"), b"not-a-manifest\n").unwrap();
    let result = Db::open(&invalid, DbOptions::default());
    assert!(
        matches!(result, Err(StorkvError::Corruption(_))),
        "invalid CURRENT must return Corruption"
    );
    let _ = fs::remove_dir_all(&invalid);
}

#[test]
fn test_manifest_manager_rejects_bad_magic_and_short_header() {
    let fs_posix = Arc::new(PosixFs);
    let root = std::env::temp_dir().join(format!("storkv_manifest_bad_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    fs::write(root.join("MANIFEST-0000000000000002"), b"SHORT").unwrap();
    fs::write(root.join("CURRENT"), b"MANIFEST-0000000000000002\n").unwrap();
    let short = ManifestManager::open(fs_posix.clone(), &root);
    assert!(matches!(short, Err(StorkvError::Corruption(_))));

    let mut header = b"BADMAGIC".to_vec();
    header.extend_from_slice(&[0u8; 8]);
    fs::write(root.join("MANIFEST-0000000000000002"), &header).unwrap();
    let bad_magic = ManifestManager::open(fs_posix.clone(), &root);
    assert!(matches!(bad_magic, Err(StorkvError::Corruption(_))));

    let _ = fs::remove_dir_all(&root);
}
