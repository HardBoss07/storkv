//! Replication cursor and snapshot operations.

mod apply;

use apply::frame_last_seq;

use crate::db::DbInner;
use crate::error::StorkvError;
use crate::replication::{
    MAX_REPLICATION_REPLAY_BYTES, MAX_REPLICATION_REPLAY_EVENTS, frame_payload_bytes,
};
use crate::vfs::FileSystem;
use crate::wal::{Durability, WalBatchFrame, WalOp, WalRecovery};
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
    /// Returns `StorkvError::WatchCursorExpired` when the retained WAL does
    /// not cover `from_seq`, `StorkvError::ReplayBudgetExceeded` when the
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

    /// Collect the live scopes and visible key/value pairs of every live scope.
    ///
    /// A primary sends this snapshot when a replica cursor expired.
    ///
    /// # Errors
    ///
    /// Returns an error when a scope scan fails.
    pub fn replication_snapshot(&self) -> crate::Result<crate::ReplicationSnapshot> {
        let _guard = self.commit_lock.lock();
        let read_seq = self.last_visible_seq.load(Ordering::Acquire);
        let now_ms = crate::types::TimestampMs::now();
        let live: Vec<(u32, std::sync::Arc<crate::scope::Scope<FS>>)> = self
            .scopes
            .read()
            .iter()
            .map(|(scope_id, scope)| (*scope_id, scope.clone()))
            .collect();
        let mut scope_ids = Vec::new();
        let mut entries = Vec::new();
        for (scope_id, scope) in live {
            if scope.is_dropped() {
                continue;
            }
            scope_ids.push(scope_id);
            for (key, value) in self.scan_prefix(scope_id, b"", read_seq)? {
                let expires_at_ms = scope
                    .key_meta(
                        key.as_ref(),
                        crate::types::SequenceNumber::new(read_seq),
                        now_ms,
                    )?
                    .map(|meta| meta.expires_at_ms)
                    .unwrap_or(0);
                entries.push((scope_id, key, value, expires_at_ms));
            }
        }
        Ok((read_seq, scope_ids, entries))
    }

    /// Apply a base snapshot from a primary.
    ///
    /// Snapshot entries are written at `snapshot_seq`. Local keys and scopes
    /// absent from the snapshot are removed, and `last_visible_seq` is advanced
    /// to `snapshot_seq`.
    ///
    /// # Errors
    ///
    /// Returns an error when a read or write fails.
    pub fn apply_replication_snapshot(
        &self,
        snapshot_seq: u64,
        scope_ids: &[u32],
        entries: &[crate::ReplicationSnapshotEntry],
    ) -> crate::Result<()> {
        let _guard = self.commit_lock.lock();
        let current_seq = self.last_visible_seq.load(Ordering::Acquire);
        if snapshot_seq <= current_seq {
            return Ok(());
        }

        let mut snapshot_keys: std::collections::HashMap<
            u32,
            std::collections::HashSet<bytes::Bytes>,
        > = std::collections::HashMap::new();
        for (scope_id, key, _value, _expires_at_ms) in entries {
            snapshot_keys
                .entry(*scope_id)
                .or_default()
                .insert(key.clone());
        }
        let snapshot_scope_ids: std::collections::HashSet<u32> =
            scope_ids.iter().copied().collect();

        let live_scope_ids: Vec<u32> = self
            .scopes
            .read()
            .iter()
            .filter(|(_, scope)| !scope.is_dropped())
            .map(|(scope_id, _)| *scope_id)
            .collect();

        for scope_id in &live_scope_ids {
            if !snapshot_scope_ids.contains(scope_id) {
                let scopes = self.scopes.read();
                let op = WalOp::DropScope {
                    scope_id: *scope_id,
                };
                crate::recovery::apply_wal_op(&scopes, op, snapshot_seq);
                continue;
            }
            let empty_keys = std::collections::HashSet::new();
            let scope_snapshot_keys = snapshot_keys.get(scope_id).unwrap_or(&empty_keys);
            let existing_entries = self.scan_prefix(*scope_id, b"", current_seq)?;
            let scopes = self.scopes.read();
            for (key, _value) in existing_entries {
                if !scope_snapshot_keys.contains(&key) {
                    let op = WalOp::Delete {
                        scope_id: *scope_id,
                        key: key.clone(),
                    };
                    crate::recovery::apply_wal_op(&scopes, op, snapshot_seq);
                }
            }
        }

        let scopes = self.scopes.read();
        for (scope_id, key, value, expires_at_ms) in entries {
            let op = WalOp::Put {
                scope_id: *scope_id,
                expires_at_ms: *expires_at_ms,
                key: key.clone(),
                val: value.clone(),
            };
            crate::recovery::apply_wal_op(&scopes, op, snapshot_seq);
        }
        drop(scopes);

        self.last_visible_seq.store(snapshot_seq, Ordering::SeqCst);
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
}
