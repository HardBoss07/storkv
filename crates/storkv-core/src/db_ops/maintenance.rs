use crate::compaction::{CompactionScheduler, Compactor, RateLimiter};
use crate::db::DbInner;
use crate::manifest::{EditRecord, VersionEdit};
use crate::scope::Scope;
use crate::types::{SequenceNumber, TimestampMs};
use crate::vfs::FileSystem;
use std::sync::Arc;
use std::sync::atomic::Ordering;

mod compact_range;

type CompactResult<FS> = crate::Result<(Arc<Scope<FS>>, Option<VersionEdit>)>;

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Pin a snapshot at the current sequence number.
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

    /// Release a pinned snapshot.
    pub fn unpin_snapshot(&self, name: &str) -> crate::Result<()> {
        let mut edit = VersionEdit::new();
        edit.add(EditRecord::UnpinSnapshot {
            name: name.to_string(),
        });
        self.manifest_mgr.log_edit(edit)
    }

    /// Return the oldest sequence number that garbage collection must keep.
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

    /// Compact every scope and persist the version edits.
    ///
    /// # Errors
    ///
    /// Returns an error when compaction or manifest logging fails.
    pub fn run_compaction(&self) -> crate::Result<()> {
        self.telemetry.record_compaction();
        let gc_safe = self.gc_safe_seq();
        let now_ms = TimestampMs::now();

        let scopes: Vec<_> = self.scopes.read().values().cloned().collect();
        for scope in &scopes {
            let edit = Compactor::compact_scope(
                scope,
                SequenceNumber::new(gc_safe),
                &self.next_file_num,
                now_ms,
                None,
            )?;
            self.apply_compaction_edit(scope, edit)?;
        }

        Ok(())
    }

    /// Compact scopes whose L0 count reached `trigger`, up to `max` scopes.
    ///
    /// The starting scope rotates across wakes so no scope starves. When more
    /// than one scope is due, their compactions run in parallel.
    ///
    /// # Errors
    ///
    /// Returns an error when compaction or manifest logging fails.
    pub fn run_due_compactions(
        &self,
        trigger: usize,
        max: usize,
        rate_limiter: Option<&RateLimiter>,
    ) -> crate::Result<usize> {
        if max == 0 {
            return Ok(0);
        }
        let gc_safe = SequenceNumber::new(self.gc_safe_seq());
        let now_ms = TimestampMs::now();
        let mut scopes: Vec<_> = self.scopes.read().values().cloned().collect();
        if scopes.is_empty() {
            return Ok(0);
        }
        let start = self.compaction_cursor.fetch_add(1, Ordering::Relaxed) % scopes.len();
        scopes.rotate_left(start);

        let due_scopes: Vec<_> = scopes
            .into_iter()
            .filter(|scope| {
                scope
                    .sst_levels
                    .read()
                    .get(&0)
                    .map(|files| files.len())
                    .unwrap_or(0)
                    >= trigger
            })
            .take(max)
            .collect();

        let results = if due_scopes.len() > 1 {
            self.compact_scopes_in_parallel(&due_scopes, gc_safe, now_ms, rate_limiter)
        } else {
            let Some(scope) = due_scopes.into_iter().next() else {
                return Ok(0);
            };
            vec![Self::compact_one(
                scope,
                gc_safe,
                now_ms,
                &self.next_file_num,
                rate_limiter,
            )]
        };

        let mut compacted = 0;
        for result in results {
            let (scope, edit) = result?;
            if self.apply_compaction_edit(&scope, edit)? {
                self.telemetry.record_compaction();
                compacted += 1;
            }
        }

        Ok(compacted)
    }

    fn compact_one(
        scope: Arc<Scope<FS>>,
        gc_safe: SequenceNumber,
        now_ms: TimestampMs,
        next_file_num: &std::sync::atomic::AtomicU64,
        rate_limiter: Option<&RateLimiter>,
    ) -> CompactResult<FS> {
        let edit = Compactor::compact_scope(&scope, gc_safe, next_file_num, now_ms, rate_limiter)?;
        Ok((scope, edit))
    }

    fn compact_scopes_in_parallel(
        &self,
        due_scopes: &[Arc<Scope<FS>>],
        gc_safe: SequenceNumber,
        now_ms: TimestampMs,
        rate_limiter: Option<&RateLimiter>,
    ) -> Vec<CompactResult<FS>> {
        let next_file_num = &self.next_file_num;
        std::thread::scope(|scope| {
            let handles: Vec<_> = due_scopes
                .iter()
                .map(|scope_arc| {
                    let scope_arc = scope_arc.clone();
                    scope.spawn(move || {
                        Self::compact_one(scope_arc, gc_safe, now_ms, next_file_num, rate_limiter)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| match handle.join() {
                    Ok(result) => result,
                    Err(_) => Err(crate::StorkvError::InvalidOperation(
                        "compaction worker panicked".into(),
                    )),
                })
                .collect()
        })
    }

    /// Wake the background scheduler after a MemTable flush.
    ///
    /// This is a no-op when background compaction is disabled.
    ///
    /// # Errors
    ///
    /// Reserved for future flush-side checks. Currently returns `Ok`.
    pub fn trigger_compaction_from_flush(&self) -> crate::Result<()> {
        if let Some(scheduler) = self.compaction_scheduler.lock().as_ref() {
            scheduler.notify();
        }
        Ok(())
    }

    /// Start the background scheduler when the option is enabled.
    ///
    /// # Errors
    ///
    /// Returns an error when the worker thread cannot start.
    pub fn start_compaction_scheduler(self: &Arc<Self>) -> crate::Result<()> {
        if !self.options.enable_background_compaction {
            return Ok(());
        }
        let mut slot = self.compaction_scheduler.lock();
        if slot.is_none() {
            *slot = Some(CompactionScheduler::start(&self.options, self)?);
        }
        Ok(())
    }

    /// Stop the background scheduler and wait for the worker to finish.
    pub fn shutdown_compaction_scheduler(&self) {
        let scheduler = self.compaction_scheduler.lock().take();
        if let Some(scheduler) = scheduler {
            scheduler.shutdown();
        }
    }

    fn apply_compaction_edit(
        &self,
        scope: &Arc<Scope<FS>>,
        edit: Option<VersionEdit>,
    ) -> crate::Result<bool> {
        let Some(edit) = edit else {
            return Ok(false);
        };
        // Persist the edit first: the manifest must never point at a file that
        // was already removed when the process restarts.
        let obsolete: Vec<u64> = edit
            .edits
            .iter()
            .filter_map(|record| match record {
                EditRecord::DeleteSstFile { file_num, .. } => Some(*file_num),
                _ => None,
            })
            .collect();
        self.manifest_mgr.log_edit(edit)?;
        for file_num in obsolete {
            self.block_cache.invalidate_sst(scope.scope_id, file_num);
            let path = scope.scope_dir.join(format!("{:016x}.sst", file_num));
            scope.fs.remove_file(&path)?;
        }
        self.notify_write_stall();
        Ok(true)
    }
}
