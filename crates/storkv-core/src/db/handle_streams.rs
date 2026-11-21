//! Watch and replication stream operations of a database handle.

use crate::watch::ChangeEvent;

use super::{Db, ScopeHandle};

impl Db {
    /// Subscribe to change events.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorkvError::TooManySubscribers`] when the watch
    /// registry is full.
    pub fn watch(&self) -> crate::Result<crossbeam_channel::Receiver<ChangeEvent>> {
        self.inner.watch_registry.subscribe()
    }

    /// Resume a change stream after `from_seq`.
    ///
    /// Retained events with a sequence number greater than `from_seq` are
    /// delivered before live events. Pass the sequence of the last processed
    /// event to resume without gaps.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorkvError::WatchCursorExpired`] when the retained
    /// log no longer covers `from_seq`.
    pub fn watch_from(
        &self,
        from_seq: u64,
    ) -> crate::Result<crossbeam_channel::Receiver<ChangeEvent>> {
        self.inner.watch_from(from_seq)
    }

    /// Resume a change stream for one scope after `from_seq`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorkvError::WatchCursorExpired`] when the retained
    /// WAL history no longer covers `from_seq`.
    pub fn watch_scope_from(
        &self,
        scope: &ScopeHandle,
        from_seq: u64,
    ) -> crate::Result<crossbeam_channel::Receiver<ChangeEvent>> {
        self.inner.watch_scope_from(scope.scope_id, from_seq)
    }

    /// Resume the replication frame stream after `from_seq`.
    ///
    /// This method is for primary nodes. It replays retained WAL frames in
    /// sequence order, then attaches the receiver to live commits.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorkvError::WatchCursorExpired`] when the retained WAL
    /// no longer covers `from_seq`, or an I/O error.
    pub fn replication_frames_from(
        &self,
        from_seq: u64,
    ) -> crate::Result<crossbeam_channel::Receiver<crate::wal::WalBatchFrame>> {
        self.inner.replication_frames_from(from_seq)
    }

    /// Apply one replicated WAL frame to this database.
    ///
    /// This method is for replica nodes. Frames that the replica already
    /// applied are ignored.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the WAL append fails.
    pub fn apply_replication_frame(&self, frame: crate::wal::WalBatchFrame) -> crate::Result<()> {
        self.inner.apply_replication_frame(frame)
    }

    /// Collect the live scopes and visible key/value pairs of every live scope.
    ///
    /// # Errors
    ///
    /// Returns an error when a scope scan fails.
    pub fn replication_snapshot(&self) -> crate::Result<crate::ReplicationSnapshot> {
        self.inner.replication_snapshot()
    }

    /// Apply a base snapshot from a primary.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when a write fails.
    pub fn apply_replication_snapshot(
        &self,
        snapshot_seq: u64,
        scopes: &[u32],
        entries: &[crate::ReplicationSnapshotEntry],
    ) -> crate::Result<()> {
        self.inner
            .apply_replication_snapshot(snapshot_seq, scopes, entries)
    }
}
