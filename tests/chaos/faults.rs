use std::fs;
use std::sync::Arc;
use storkv_core::DbOptions;
use storkv_core::vfs::{FaultInjectingFs, PosixFs};
use storkv_core::wal::WalManager;

#[test]
fn test_fault_injected_write_and_sync_failures_propagate_from_wal() {
    let root = std::env::temp_dir().join(format!("storkv_fault_wal_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    let fs_faulty = Arc::new(FaultInjectingFs::new(PosixFs));
    let fs_handle = Arc::new(fs_faulty.clone());

    let sync_dir = root.join("sync_fail");
    fs::create_dir_all(&sync_dir).unwrap();
    fs_faulty.set_fail_syncs(true);
    let sync_failure = WalManager::open(fs_handle.clone(), &sync_dir, 1);
    assert!(sync_failure.is_err(), "sync failure must abort open");
    fs_faulty.set_fail_syncs(false);

    let write_dir = root.join("write_fail");
    fs::create_dir_all(&write_dir).unwrap();
    fs_faulty.set_fail_writes(true);
    let write_failure = WalManager::open(fs_handle.clone(), &write_dir, 1);
    assert!(write_failure.is_err(), "write failure must abort open");
    fs_faulty.set_fail_writes(false);

    let healthy_dir = root.join("healthy");
    fs::create_dir_all(&healthy_dir).unwrap();
    let healthy = WalManager::open(fs_handle.clone(), &healthy_dir, 1);
    assert!(healthy.is_ok(), "recovered filesystem must open");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_fault_injected_sync_failure_aborts_db_open() {
    let root = std::env::temp_dir().join(format!("storkv_fault_db_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    let fs_faulty = Arc::new(FaultInjectingFs::new(PosixFs));
    let fs_handle = Arc::new(fs_faulty.clone());
    fs_faulty.set_fail_syncs(true);
    let result = storkv_core::DbInner::open(fs_handle.clone(), root.clone(), DbOptions::default());
    assert!(
        result.is_err(),
        "injected sync failure during DbInner::open must propagate"
    );

    let _ = fs::remove_dir_all(&root);
}
