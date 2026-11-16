use bytes::Bytes;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use storkv_core::wal::{Durability, WalBatchFrame, WalManager, WalOp, WalRecovery};

#[test]
fn test_16_thread_concurrent_group_commit_coalescing() {
    let temp = std::env::temp_dir().join(format!("storkv_wal_concurrent_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();

    let fs_posix = Arc::new(storkv_core::vfs::PosixFs);
    let manager = Arc::new(WalManager::open(fs_posix.clone(), &temp, 1).unwrap());

    let threads = 16;
    let batches_per_thread = 20;
    let mut handles = Vec::new();
    for thread_id in 0..threads {
        let manager = manager.clone();
        handles.push(std::thread::spawn(move || {
            for batch in 0..batches_per_thread {
                let sequence = (thread_id * batches_per_thread + batch) as u64;
                let frame = WalBatchFrame::new(
                    sequence,
                    vec![WalOp::Put {
                        scope_id: 0,
                        expires_at_ms: 0,
                        key: Bytes::from(format!("k_{thread_id}_{batch}")),
                        val: Bytes::from(format!("v_{thread_id}_{batch}")),
                    }],
                );
                manager
                    .write_batch(
                        &frame,
                        Durability::Buffered {
                            sync_interval: Duration::from_millis(10),
                        },
                    )
                    .unwrap();
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }
    drop(manager);

    let mut recovered = Vec::new();
    for entry in fs::read_dir(&temp).unwrap().flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("wal") {
            recovered.extend(
                WalRecovery::recover_segment(fs_posix.clone(), &path, true)
                    .expect("concurrent WAL recovery"),
            );
        }
    }
    let op_count: usize = recovered.iter().map(|frame| frame.ops.len()).sum();
    assert_eq!(op_count, threads * batches_per_thread);

    let mut sequences: Vec<u64> = recovered.iter().map(|frame| frame.batch_seq).collect();
    sequences.sort_unstable();
    assert_eq!(
        sequences,
        (0..(threads * batches_per_thread) as u64).collect::<Vec<_>>()
    );

    let _ = fs::remove_dir_all(&temp);
}
