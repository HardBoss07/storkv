use crate::db::DbInner;
use crate::manifest::{EditRecord, VersionEdit};
use crate::types::{SequenceNumber, TimestampMs};
use crate::vfs::FileSystem;
use std::sync::atomic::Ordering;

impl<FS: FileSystem + 'static> DbInner<FS> {
    pub fn create_snapshot(&self, name: String) -> crate::Result<()> {
        let seq = self.last_visible_seq.load(Ordering::Acquire);
        let mut edit = VersionEdit::new();
        edit.add(EditRecord::PinSnapshot {
            seq,
            created_at_ms: TimestampMs::now().get(),
            name,
        });
        self.manifest_mgr.log_edit(edit)
    }

    pub fn unpin_snapshot(&self, name: &str) -> crate::Result<()> {
        let mut edit = VersionEdit::new();
        edit.add(EditRecord::UnpinSnapshot {
            name: name.to_string(),
        });
        self.manifest_mgr.log_edit(edit)
    }

    pub fn gc_safe_seq(&self) -> u64 {
        let readers = self.active_tx_readers.read();
        let oldest_transaction = readers.iter().min().copied().unwrap_or(u64::MAX);

        let state = self.manifest_mgr.state();
        let oldest_snapshot = state
            .pinned_snapshots
            .values()
            .map(|snapshot| snapshot.seq)
            .min()
            .unwrap_or(u64::MAX);

        oldest_transaction
            .min(oldest_snapshot)
            .min(self.last_visible_seq.load(Ordering::Acquire))
    }

    pub fn run_compaction(&self) -> crate::Result<()> {
        self.telemetry.record_compaction();
        let gc_safe = self.gc_safe_seq();
        let now_ms = TimestampMs::now();

        let scopes: Vec<_> = self.scopes.read().values().cloned().collect();
        for scope in &scopes {
            if let Some(edit) = crate::compaction::Compactor::compact_scope(
                scope,
                SequenceNumber::new(gc_safe),
                &self.next_file_num,
                now_ms,
            )? {
                self.manifest_mgr.log_edit(edit)?;
            }
        }

        Ok(())
    }
}
