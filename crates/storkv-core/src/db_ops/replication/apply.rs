//! Apply replicated WAL frames to scope state and watch subscribers.

use crate::db::DbInner;
use crate::vfs::FileSystem;
use crate::wal::{WalBatchFrame, WalOp};
use crate::watch::ChangeEvent;

impl<FS: FileSystem + 'static> DbInner<FS> {
    pub(super) fn apply_frame_ops(&self, frame: &WalBatchFrame, previous_seq: u64) {
        let scopes = self.scopes.read();
        for (index, op) in frame.ops.iter().enumerate() {
            let op_seq = frame.batch_seq + index as u64;
            if op_seq <= previous_seq {
                continue;
            }
            crate::recovery::apply_wal_op(&scopes, op.clone(), op_seq);
        }
        drop(scopes);
        for (index, op) in frame.ops.iter().enumerate() {
            let op_seq = frame.batch_seq + index as u64;
            if op_seq > previous_seq
                && let Some(event) = watch_event_for_op(op, op_seq)
            {
                self.watch_registry.broadcast(event);
            }
        }
    }
}

/// Highest sequence number that a frame covers, if the frame is not empty.
pub(super) fn frame_last_seq(frame: &WalBatchFrame) -> Option<u64> {
    let count = frame.ops.len() as u64;
    (count > 0).then(|| frame.batch_seq.saturating_add(count - 1))
}

fn watch_event_for_op(op: &WalOp, seq: u64) -> Option<ChangeEvent> {
    match op {
        WalOp::Put {
            scope_id,
            key,
            val,
            expires_at_ms,
        } => Some(ChangeEvent::Put {
            scope_id: *scope_id,
            key: key.clone(),
            value: val.clone(),
            seq,
            expires_at_ms: *expires_at_ms,
        }),
        WalOp::Delete { scope_id, key } => Some(ChangeEvent::Delete {
            scope_id: *scope_id,
            key: key.clone(),
            seq,
        }),
        WalOp::DropScope { scope_id } => Some(ChangeEvent::DropScope {
            scope_id: *scope_id,
            seq,
        }),
        WalOp::Merge { scope_id, .. } | WalOp::DeleteRange { scope_id, .. } => {
            Some(ChangeEvent::InvalidateScope {
                scope_id: *scope_id,
                seq,
            })
        }
    }
}
