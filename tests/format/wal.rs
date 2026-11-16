use bytes::Bytes;
use std::fs;
use std::sync::Arc;
use storkv_core::format::compute_crc32c;
use storkv_core::vfs::PosixFs;
use storkv_core::wal::{
    Durability, WAL_FRAME_HEADER_SIZE, WAL_HEADER_SIZE, WAL_MAGIC, WalBatchFrame, WalManager, WalOp,
};

#[test]
fn test_wal_header_and_batch_frame_binary_layout() {
    let temp = std::env::temp_dir().join(format!("storkv_wal_layout_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();

    let fs_posix = Arc::new(PosixFs);
    let wal_manager = WalManager::open(fs_posix.clone(), &temp, 1).unwrap();

    let ops = vec![
        WalOp::Put {
            scope_id: 1,
            expires_at_ms: 5000,
            key: Bytes::from_static(b"k1"),
            val: Bytes::from_static(b"v1"),
        },
        WalOp::Delete {
            scope_id: 1,
            key: Bytes::from_static(b"k2"),
        },
        WalOp::Merge {
            scope_id: 1,
            key: Bytes::from_static(b"k3"),
            val: Bytes::from_static(b"1"),
        },
        WalOp::DeleteRange {
            scope_id: 1,
            start_key: Bytes::from_static(b"k4"),
            end_key: Bytes::from_static(b"k5"),
        },
        WalOp::DropScope { scope_id: 1 },
    ];

    let frame = WalBatchFrame::new(100, ops);
    wal_manager
        .write_batch(&frame, Durability::StrictFsync)
        .unwrap();
    drop(wal_manager);

    let wal_path = temp.join("0000000000000001.wal");
    let bytes = fs::read(&wal_path).unwrap();

    assert!(bytes.len() >= WAL_HEADER_SIZE);
    assert_eq!(&bytes[0..8], WAL_MAGIC);
    let version = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
    assert_eq!(version, 1);
    let flags = u16::from_le_bytes(bytes[10..12].try_into().unwrap());
    assert_eq!(flags, 0);

    let frame_bytes = &bytes[WAL_HEADER_SIZE..];
    assert!(frame_bytes.len() >= WAL_FRAME_HEADER_SIZE);
    let checksum = u32::from_le_bytes(frame_bytes[0..4].try_into().unwrap());
    let frame_length = u32::from_le_bytes(frame_bytes[4..8].try_into().unwrap());
    let batch_seq = u64::from_le_bytes(frame_bytes[8..16].try_into().unwrap());
    let op_count = u32::from_le_bytes(frame_bytes[16..20].try_into().unwrap());
    let frame_flags = u16::from_le_bytes(frame_bytes[20..22].try_into().unwrap());

    assert_eq!(batch_seq, 100);
    assert_eq!(op_count, 5);
    assert_eq!(frame_flags, 0);

    let computed_checksum = compute_crc32c(&frame_bytes[4..8 + frame_length as usize]);
    assert_eq!(checksum, computed_checksum);

    let _ = fs::remove_dir_all(&temp);
}
