use std::fs;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use storkv_core::wal::{WAL_HEADER_SIZE, WAL_MAGIC, WalBatchFrame, WalOp};

pub(crate) fn backup_root(temp: &crate::common::TempDb) -> PathBuf {
    temp.path.join("backups")
}

pub(crate) fn generation_directories(root: &Path) -> Vec<PathBuf> {
    let generations = root.join("generations");
    let Ok(entries) = fs::read_dir(&generations) else {
        return Vec::new();
    };
    let mut directories: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    directories.sort();
    directories
}

/// Write a synthetic sealed WAL segment into `<backup_root>/wal`.
pub(crate) fn write_wal_segment(
    backup_root: &Path,
    file_num: u64,
    frame: WalBatchFrame,
) -> PathBuf {
    let wal_dir = backup_root.join("wal");
    fs::create_dir_all(&wal_dir).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(WAL_MAGIC);
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(bytes.len(), WAL_HEADER_SIZE);
    bytes.extend_from_slice(&frame.encode(false).unwrap());

    let path = wal_dir.join(format!("{file_num:016x}.wal"));
    fs::write(&path, &bytes).unwrap();
    path
}

pub(crate) fn put_op(scope_id: u32, key: &str, value: &str) -> WalOp {
    WalOp::Put {
        scope_id,
        expires_at_ms: 0,
        key: Bytes::from(key.to_string()),
        val: Bytes::from(value.to_string()),
    }
}
