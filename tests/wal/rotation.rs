use bytes::Bytes;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use storkv_core::wal::{Durability, WAL_MAGIC, WalBatchFrame, WalManager, WalOp};

#[test]
fn test_wal_segment_rotation_and_sealed_flag() {
    let temp = std::env::temp_dir().join(format!("storkv_wal_rot_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();

    let fs_posix = Arc::new(storkv_core::vfs::PosixFs);
    let wal_manager = WalManager::open(fs_posix.clone(), &temp, 1).unwrap();

    assert_eq!(wal_manager.current_file_num(), 1);

    let big_value = vec![b'x'; 1024 * 1024];
    for index in 0..33 {
        let frame = WalBatchFrame::new(
            (index * 10) as u64,
            vec![WalOp::Put {
                scope_id: 0,
                expires_at_ms: 0,
                key: Bytes::from(format!("key_{}", index)),
                val: Bytes::copy_from_slice(&big_value),
            }],
        );
        wal_manager
            .write_batch(
                &frame,
                Durability::Buffered {
                    sync_interval: Duration::from_millis(100),
                },
            )
            .unwrap();
    }

    assert!(
        wal_manager.current_file_num() > 1,
        "WAL segment should have rotated after 33 MiB of writes"
    );

    drop(wal_manager);

    let segment_path = temp.join("0000000000000001.wal");
    let segment_bytes = fs::read(&segment_path).unwrap();
    assert_eq!(&segment_bytes[0..8], WAL_MAGIC);
    let flags = u16::from_le_bytes(segment_bytes[10..12].try_into().unwrap());
    assert_eq!(
        flags & 0x0001,
        0x0001,
        "Sealed segment must have bit 0x0001 set in header flags"
    );

    let _ = fs::remove_dir_all(&temp);
}
