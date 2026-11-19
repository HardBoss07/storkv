use crate::db::DbInner;
use crate::error::StorkvError;
use crate::replication::{
    MAX_REPLICATION_REPLAY_BYTES, MAX_REPLICATION_REPLAY_EVENTS, frame_payload_bytes,
};
use crate::vfs::FileSystem;
use crate::wal::{Durability, WalBatchFrame, WalOp, WalRecovery};
use crate::watch::ChangeEvent;
use crossbeam_channel::Receiver;
use std::sync::atomic::Ordering;

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Resume the replication frame stream after `from_seq`.
    ///
    /// The method replays retained WAL frames in sequence order, then attaches
    /// the receiver to live commits. The commit lock makes the transition
    /// gapless.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::WatchCursorExpired`] when the retained WAL does
    /// not cover `from_seq`, [`StorkvError::ReplayBudgetExceeded`] when the
    /// replay is too large, or an I/O error.
    pub fn replication_frames_from(&self, from_seq: u64) -> crate::Result<Receiver<WalBatchFrame>> {
        let _guard = self.commit_lock.lock();
        let newest = self.last_visible_seq.load(Ordering::Acquire);
        let (oldest, replay) = self.collect_frames(from_seq)?;
        let oldest_available = oldest.unwrap_or_else(|| newest.saturating_add(1));
        if from_seq < newest && from_seq.saturating_add(1) < oldest_available {
            return Err(StorkvError::WatchCursorExpired {
                requested: from_seq,
                oldest_available,
            });
        }
        self.replication_registry.subscribe_replaying(replay)
    }

    /// Apply one replicated WAL frame in sequence order.
    ///
    /// The frame is appended to the local WAL and then applied to the scope
    /// memtables. Frames that the replica already applied are ignored.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the WAL append fails.
    pub fn apply_replication_frame(&self, frame: WalBatchFrame) -> crate::Result<()> {
        let _guard = self.commit_lock.lock();
        let Some(last_seq) = frame_last_seq(&frame) else {
            return Ok(());
        };
        let current = self.last_visible_seq.load(Ordering::Acquire);
        if last_seq <= current {
            return Ok(());
        }
        self.wal_mgr.write_batch(&frame, Durability::StrictFsync)?;
        let tombstones = frame
            .ops
            .iter()
            .enumerate()
            .filter_map(|(index, op)| match op {
                WalOp::DeleteRange {
                    scope_id,
                    start_key,
                    end_key,
                } => Some((
                    *scope_id,
                    frame.batch_seq + index as u64,
                    start_key.clone(),
                    end_key.clone(),
                )),
                _ => None,
            });
        self.log_range_tombstones(tombstones)?;
        self.apply_frame_ops(&frame, current);
        self.last_visible_seq.store(last_seq, Ordering::SeqCst);
        Ok(())
    }

    /// Collect the visible key/value pairs of every live scope.
    ///
    /// A primary sends this snapshot when a replica cursor expired.
    ///
    /// # Errors
    ///
    /// Returns an error when a scope scan fails.
    pub fn replication_snapshot(&self) -> crate::Result<Vec<(u32, bytes::Bytes, bytes::Bytes)>> {
        let _guard = self.commit_lock.lock();
        let read_seq = self.last_visible_seq.load(Ordering::Acquire);
        let live: Vec<(u32, std::sync::Arc<crate::scope::Scope<FS>>)> = self
            .scopes
            .read()
            .iter()
            .map(|(scope_id, scope)| (*scope_id, scope.clone()))
            .collect();
        let mut entries = Vec::new();
        for (scope_id, scope) in live {
            if scope.is_dropped() {
                continue;
            }
            for (key, value) in self.scan_prefix(scope_id, b"", read_seq)? {
                entries.push((scope_id, key, value));
            }
        }
        Ok(entries)
    }

    /// Apply a base snapshot from a primary.
    ///
    /// Snapshot entries are written at the current sequence number. Later
    /// frames apply on top of the snapshot.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when a write fails.
    pub fn apply_replication_snapshot(
        &self,
        entries: &[(u32, bytes::Bytes, bytes::Bytes)],
    ) -> crate::Result<()> {
        let _guard = self.commit_lock.lock();
        let read_seq = self.last_visible_seq.load(Ordering::Acquire);
        let scopes = self.scopes.read();
        for (scope_id, key, value) in entries {
            let op = WalOp::Put {
                scope_id: *scope_id,
                expires_at_ms: 0,
                key: key.clone(),
                val: value.clone(),
            };
            crate::recovery::apply_wal_op(&scopes, op, read_seq);
        }
        Ok(())
    }

    fn collect_frames(&self, from_seq: u64) -> crate::Result<(Option<u64>, Vec<WalBatchFrame>)> {
        let mut segments = crate::recovery::scan_wal_segments(self.fs.as_ref(), &self.wal_dir);
        segments.sort_unstable();
        let mut oldest = None;
        let mut replay = Vec::new();
        let mut replay_bytes = 0usize;
        for (index, &segment_number) in segments.iter().enumerate() {
            let path = self.wal_dir.join(format!("{:016x}.wal", segment_number));
            let is_last = index + 1 == segments.len();
            let frames = WalRecovery::recover_segment(self.fs.clone(), &path, is_last)?;
            for frame in frames {
                oldest.get_or_insert(frame.batch_seq);
                if frame.batch_seq >= from_seq {
                    replay_bytes = replay_bytes.saturating_add(frame_payload_bytes(&frame));
                    if replay.len() >= MAX_REPLICATION_REPLAY_EVENTS
                        || replay_bytes > MAX_REPLICATION_REPLAY_BYTES
                    {
                        return Err(StorkvError::ReplayBudgetExceeded {
                            events: replay.len() + 1,
                            bytes: replay_bytes,
                        });
                    }
                    replay.push(frame);
                }
            }
        }
        Ok((oldest, replay))
    }

    fn apply_frame_ops(&self, frame: &WalBatchFrame, previous_seq: u64) {
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

fn frame_last_seq(frame: &WalBatchFrame) -> Option<u64> {
    let count = frame.ops.len() as u64;
    (count > 0).then(|| frame.batch_seq.saturating_add(count - 1))
}

fn watch_event_for_op(op: &WalOp, seq: u64) -> Option<ChangeEvent> {
    match op {
        WalOp::Put {
            scope_id, key, val, ..
        } => Some(ChangeEvent::Put {
            scope_id: *scope_id,
            key: key.clone(),
            value: val.clone(),
            seq,
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
