//! WAL replay and in-memory application of WAL operations.
//!
//! Replay skips operations at or below the scope snapshot watermark. This
//! rule prevents duplicate merge operands after a reopen.

use crate::format::{InternalKey, ValueType};
use crate::scope::{Scope, StorageMode};
use crate::vfs::FileSystem;
use crate::wal::{WalOp, WalRecovery};
use bytes::Bytes;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub(crate) fn replay_wal<FS: FileSystem + 'static>(
    fs: Arc<FS>,
    wal_dir: &Path,
    scopes: &HashMap<u32, Arc<Scope<FS>>>,
    mut max_seq: u64,
) -> u64 {
    let mut segment_numbers = scan_wal_segments(&*fs, wal_dir);
    segment_numbers.sort();

    for (index, &segment_number) in segment_numbers.iter().enumerate() {
        let segment_path = wal_dir.join(format!("{:016x}.wal", segment_number));
        let is_last_segment = index + 1 == segment_numbers.len();
        if let Ok(frames) = WalRecovery::recover_segment(fs.clone(), &segment_path, is_last_segment)
        {
            for frame in frames {
                for (op_index, op) in frame.ops.into_iter().enumerate() {
                    let op_seq = frame.batch_seq + op_index as u64;
                    max_seq = max_seq.max(op_seq);
                    apply_wal_op(scopes, op, op_seq);
                }
            }
        }
    }
    max_seq
}

pub(crate) fn scan_wal_segments<FS: FileSystem>(fs: &FS, wal_dir: &Path) -> Vec<u64> {
    fs.read_dir(wal_dir)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|path| {
            let file_name = path.file_name()?.to_str()?;
            let hex_number = file_name.strip_suffix(".wal")?;
            u64::from_str_radix(hex_number, 16).ok()
        })
        .collect()
}

pub(crate) fn apply_wal_op<FS: FileSystem>(
    scopes: &HashMap<u32, Arc<Scope<FS>>>,
    op: WalOp,
    op_seq: u64,
) {
    let Some(scope) = scopes.get(&scope_id_of(&op)) else {
        return;
    };
    if scope.options.storage_mode == StorageMode::InMemory {
        return;
    }
    if op_seq <= scope.snapshot_seq.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }

    match op {
        WalOp::Put {
            expires_at_ms,
            key,
            val,
            ..
        } => {
            let internal_key = InternalKey::new(key, op_seq, ValueType::Value, expires_at_ms);
            scope
                .active_memtable
                .read()
                .insert(internal_key.encode(), val, op_seq);
        }
        WalOp::Delete { key, .. } => {
            let internal_key = InternalKey::new(key, op_seq, ValueType::Tombstone, 0);
            scope
                .active_memtable
                .read()
                .insert(internal_key.encode(), Bytes::new(), op_seq);
        }
        WalOp::Merge { key, val, .. } => {
            let internal_key = InternalKey::new(key, op_seq, ValueType::MergeOperand, 0);
            scope
                .active_memtable
                .read()
                .insert(internal_key.encode(), val, op_seq);
        }
        WalOp::DeleteRange {
            start_key, end_key, ..
        } => {
            scope.record_range_tombstone(start_key.clone(), end_key.clone(), op_seq);
            let internal_key = InternalKey::new(start_key, op_seq, ValueType::RangeTombstone, 0);
            scope
                .active_memtable
                .read()
                .insert(internal_key.encode(), end_key, op_seq);
        }
        WalOp::DropScope { .. } => scope.mark_dropped(op_seq),
    }
}

fn scope_id_of(op: &WalOp) -> u32 {
    match op {
        WalOp::Put { scope_id, .. }
        | WalOp::Delete { scope_id, .. }
        | WalOp::Merge { scope_id, .. }
        | WalOp::DeleteRange { scope_id, .. }
        | WalOp::DropScope { scope_id } => *scope_id,
    }
}
