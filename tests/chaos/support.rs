use bytes::Bytes;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use storkv_core::vfs::PosixFs;
use storkv_core::wal::{Durability, WalBatchFrame, WalManager, WalOp};

pub(super) fn write_two_frame_wal(directory: &Path) -> PathBuf {
    let fs_posix = Arc::new(PosixFs);
    let manager = WalManager::open(fs_posix, directory, 1).unwrap();
    for batch in 0..2u64 {
        let frame = WalBatchFrame::new(
            batch + 1,
            vec![WalOp::Put {
                scope_id: 0,
                expires_at_ms: 0,
                key: Bytes::from(format!("chaos_key_{batch}")),
                val: Bytes::from(format!("chaos_val_{batch}")),
            }],
        );
        manager
            .write_batch(&frame, Durability::StrictFsync)
            .unwrap();
    }
    directory.join("0000000000000001.wal")
}
