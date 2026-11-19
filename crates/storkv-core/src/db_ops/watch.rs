use crate::db::DbInner;
use crate::error::StorkvError;
use crate::vfs::FileSystem;
use crate::wal::{WalOp, WalRecovery};
use crate::watch::{ChangeEvent, MAX_REPLAY_BYTES, MAX_REPLAY_EVENTS};
use crossbeam_channel::Receiver;
use std::sync::atomic::Ordering;

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Resume a change stream after `from_seq`.
    ///
    /// The method replays retained WAL events with a sequence number at or
    /// after `from_seq`, then attaches the receiver to live broadcasts. The
    /// commit lock keeps the replay-to-live transition gapless.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::WatchCursorExpired`] when `from_seq` predates the
    /// oldest retained WAL event, [`StorkvError::ReplayBudgetExceeded`] when the
    /// replay is too large, or an I/O error.
    pub fn watch_from(&self, from_seq: u64) -> crate::Result<Receiver<ChangeEvent>> {
        self.watch_filtered(from_seq, None)
    }

    /// Resume a change stream for one scope after `from_seq`.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::WatchCursorExpired`] when `from_seq` predates the
    /// oldest retained WAL event, [`StorkvError::ReplayBudgetExceeded`] when the
    /// replay is too large, or an I/O error.
    pub fn watch_scope_from(
        &self,
        scope_id: u32,
        from_seq: u64,
    ) -> crate::Result<Receiver<ChangeEvent>> {
        self.watch_filtered(from_seq, Some(scope_id))
    }

    fn watch_filtered(
        &self,
        from_seq: u64,
        scope_id: Option<u32>,
    ) -> crate::Result<Receiver<ChangeEvent>> {
        let _guard = self.commit_lock.lock();
        let newest = self.last_visible_seq.load(Ordering::Acquire);
        let (oldest, replay) = self.collect_watch_events(from_seq, scope_id)?;
        let oldest_available = oldest.unwrap_or_else(|| newest.saturating_add(1));
        if from_seq < newest && from_seq.saturating_add(1) < oldest_available {
            return Err(StorkvError::WatchCursorExpired {
                requested: from_seq,
                oldest_available,
            });
        }
        self.watch_registry.subscribe_replaying(replay)
    }

    fn collect_watch_events(
        &self,
        from_seq: u64,
        scope_id: Option<u32>,
    ) -> crate::Result<(Option<u64>, Vec<ChangeEvent>)> {
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
                for (op_index, op) in frame.ops.into_iter().enumerate() {
                    let seq = frame.batch_seq + op_index as u64;
                    let Some(event) = change_event(op, seq) else {
                        continue;
                    };
                    let in_scope = scope_id.is_none_or(|id| event.scope_id() == id);
                    if seq >= from_seq && in_scope {
                        replay_bytes = replay_bytes.saturating_add(event.payload_bytes());
                        if replay.len() >= MAX_REPLAY_EVENTS || replay_bytes > MAX_REPLAY_BYTES {
                            return Err(StorkvError::ReplayBudgetExceeded {
                                events: replay.len() + 1,
                                bytes: replay_bytes,
                            });
                        }
                        replay.push(event);
                    }
                }
            }
        }
        Ok((oldest, replay))
    }
}

fn change_event(op: WalOp, seq: u64) -> Option<ChangeEvent> {
    match op {
        WalOp::Put {
            scope_id, key, val, ..
        } => Some(ChangeEvent::Put {
            scope_id,
            key,
            value: val,
            seq,
        }),
        WalOp::Delete { scope_id, key } => Some(ChangeEvent::Delete { scope_id, key, seq }),
        WalOp::DropScope { scope_id } => Some(ChangeEvent::DropScope { scope_id, seq }),
        WalOp::Merge { scope_id, .. } | WalOp::DeleteRange { scope_id, .. } => {
            Some(ChangeEvent::InvalidateScope { scope_id, seq })
        }
    }
}
