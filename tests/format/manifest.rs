use bytes::Bytes;
use std::fs;
use std::sync::Arc;
use storkv_core::manifest::{EditRecord, ManifestManager, SstFileMeta, VersionEdit};
use storkv_core::vfs::PosixFs;

#[test]
fn test_manifest_header_and_all_eight_edit_records() {
    let temp = std::env::temp_dir().join(format!("storkv_mnf_all_edits_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();

    let fs_posix = Arc::new(PosixFs);
    let manager = ManifestManager::open(fs_posix.clone(), &temp).unwrap();

    let mut edit = VersionEdit::new();
    edit.add(EditRecord::SetNextSequence(1000));
    edit.add(EditRecord::SetWalNumber(42));
    edit.add(EditRecord::CreateScope {
        scope_id: 10,
        durability: 0,
        sync_ms: 100,
        retention: 1,
        default_ttl_ms: 60000,
        compaction_style: 1,
        storage_mode: 0,
        name: "test_scope_10".into(),
    });
    edit.add(EditRecord::DropScope {
        scope_id: 10,
        drop_seq: 1005,
    });
    edit.add(EditRecord::AddSstFile(SstFileMeta {
        scope_id: 10,
        level: 1,
        file_num: 99,
        file_size: 8192,
        min_seq: 50,
        max_seq: 90,
        max_expires_at_ms: 12345678,
        min_key: Bytes::from_static(b"k_start"),
        max_key: Bytes::from_static(b"k_end"),
    }));
    edit.add(EditRecord::DeleteSstFile {
        scope_id: 10,
        level: 1,
        file_num: 99,
    });
    edit.add(EditRecord::PinSnapshot {
        seq: 500,
        created_at_ms: 999999,
        name: "snap_important".into(),
    });
    edit.add(EditRecord::UnpinSnapshot {
        name: "snap_important".into(),
    });

    manager.log_edit(edit).unwrap();
    drop(manager);

    let reopened = ManifestManager::open(fs_posix.clone(), &temp).unwrap();
    let state = reopened.state();
    assert_eq!(state.next_seq, 1000);
    assert_eq!(state.min_unflushed_wal_num, 42);
    let scope_meta = state
        .scopes
        .get(&10)
        .expect("Scope 10 must exist in manifest");
    assert!(scope_meta.dropped);
    assert_eq!(scope_meta.drop_seq, 1005);
    assert!(!state.sst_files.contains_key(&(10, 1, 99)));
    assert!(!state.pinned_snapshots.contains_key("snap_important"));

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn test_manifest_corruption_detected_at_every_byte() {
    let mut edit = VersionEdit::new();
    edit.add(EditRecord::SetNextSequence(77));
    edit.add(EditRecord::PinSnapshot {
        seq: 77,
        created_at_ms: 1234,
        name: "snap".into(),
    });
    let encoded = edit.encode();
    for offset in 0..encoded.len() {
        let mut corrupted = encoded.clone();
        corrupted[offset] ^= 0x01;
        assert!(
            VersionEdit::decode(&corrupted).is_err(),
            "manifest corruption at offset {offset} must be rejected"
        );
    }
}
