use bytes::Bytes;
use std::fs;
use std::io::{Read, Write};
use std::sync::Arc;
use storkv_core::StorKvError;
use storkv_core::vfs::{FileSystem, PosixFs};
use storkv_core::wal::{
    Durability, WAL_HEADER_SIZE, WalBatchFrame, WalManager, WalOp, WalRecovery,
};

use super::support::write_two_frame_wal;

#[test]
fn test_torn_wal_tail_truncation_vs_mid_segment_corruption() {
    let temp = std::env::temp_dir().join(format!("storkv_wal_chaos_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();

    let fs_posix = Arc::new(PosixFs);
    let wal_manager = WalManager::open(fs_posix.clone(), &temp, 1).unwrap();

    let ops = vec![WalOp::Put {
        scope_id: 0,
        expires_at_ms: 0,
        key: Bytes::from_static(b"chaos_k1"),
        val: Bytes::from_static(b"chaos_v1"),
    }];
    wal_manager
        .write_batch(&WalBatchFrame::new(1, ops), Durability::StrictFsync)
        .unwrap();
    drop(wal_manager);

    let wal_path = temp.join("0000000000000001.wal");

    {
        let mut file = fs_posix.append_file(&wal_path).unwrap();
        file.write_all(b"incomplete_torn_trailing_bytes_from_power_loss")
            .unwrap();
        file.sync_all().unwrap();
    }

    let recovered = WalRecovery::recover_segment(fs_posix.clone(), &wal_path, true)
        .expect("Torn tail recovery should succeed");
    assert_eq!(recovered.len(), 1);

    {
        let mut file = fs_posix.open_file(&wal_path).unwrap();
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();
        data[22] ^= 0xFF;
        let mut write_file = fs_posix.create_file(&wal_path).unwrap();
        write_file.write_all(&data).unwrap();
        write_file.sync_all().unwrap();
    }

    let error = WalRecovery::recover_segment(fs_posix.clone(), &wal_path, false);
    assert!(
        matches!(error, Err(StorKvError::Corruption(_))),
        "Mid-segment corruption must fail with StorKvError::Corruption"
    );

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn test_wal_every_byte_flip_recovers_without_panic() {
    let temp = std::env::temp_dir().join(format!("storkv_wal_bitrot_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();

    let fs_posix = Arc::new(PosixFs);
    let wal_path = write_two_frame_wal(&temp);
    let original = fs::read(&wal_path).unwrap();
    assert!(original.len() > WAL_HEADER_SIZE);

    for offset in WAL_HEADER_SIZE..original.len() {
        let mut corrupted = original.clone();
        corrupted[offset] ^= 0x01;
        fs::write(&wal_path, &corrupted).unwrap();

        let last = WalRecovery::recover_segment(fs_posix.clone(), &wal_path, true)
            .expect("last-segment recovery must not panic or hard-fail");
        assert!(last.len() <= 2);

        let interior = WalRecovery::recover_segment(fs_posix.clone(), &wal_path, false);
        match interior {
            Ok(frames) => assert!(frames.len() <= 2),
            Err(StorKvError::Corruption(_)) => {}
            Err(other) => panic!("unexpected error for offset {offset}: {other:?}"),
        }
    }

    fs::write(&wal_path, &original).unwrap();
    assert_eq!(
        WalRecovery::recover_segment(fs_posix.clone(), &wal_path, true)
            .unwrap()
            .len(),
        2
    );

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn test_wal_every_truncation_point_recovers_without_panic() {
    let temp = std::env::temp_dir().join(format!("storkv_wal_truncate_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();

    let fs_posix = Arc::new(PosixFs);
    let wal_path = write_two_frame_wal(&temp);
    let original = fs::read(&wal_path).unwrap();

    for length in 0..=original.len() {
        fs::write(&wal_path, &original[..length]).unwrap();
        let last = WalRecovery::recover_segment(fs_posix.clone(), &wal_path, true);
        assert!(last.is_ok(), "last-segment prefix {length} must recover");
        let interior = WalRecovery::recover_segment(fs_posix.clone(), &wal_path, false);
        if length < WAL_HEADER_SIZE {
            assert!(
                matches!(interior, Err(StorKvError::Corruption(_))),
                "short interior segment {length} must be corruption"
            );
        } else {
            assert!(interior.is_ok() || matches!(interior, Err(StorKvError::Corruption(_))));
        }
    }

    let _ = fs::remove_dir_all(&temp);
}
