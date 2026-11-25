//! Foreground write backpressure for L0 file counts and un-flushed WAL bytes.

use crate::db::DbInner;
use crate::options::WriteOptions;
use crate::scope::Scope;
use crate::vfs::FileSystem;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Largest progressive delay injected before a write at the slowdown trigger.
const MAX_WRITE_SLOWDOWN_DELAY_MS: u64 = 20;
/// How long a stalled writer waits before it rechecks the watermarks.
const STALL_POLL_INTERVAL: Duration = Duration::from_millis(10);
/// How long a stalled writer waits before it drains the backlog itself.
const STALL_RELIEF_INTERVAL: Duration = Duration::from_secs(2);

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Add MemTable bytes to the un-flushed WAL estimate.
    pub(crate) fn record_unflushed_wal_bytes(&self, scope: &Scope<FS>, bytes: usize) {
        if scope.options.storage_mode != crate::StorageMode::InMemory {
            self.unflushed_wal_bytes
                .fetch_add(bytes as u64, Ordering::Relaxed);
        }
    }

    /// Remove the bytes of a MemTable that a flush drained.
    pub(crate) fn drain_unflushed_wal_bytes(&self, bytes: usize) {
        let bytes = bytes as u64;
        let mut current = self.unflushed_wal_bytes.load(Ordering::Acquire);
        loop {
            let next = current.saturating_sub(bytes);
            match self.unflushed_wal_bytes.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }

    /// Wake every writer that waits for backlog to drain.
    pub(crate) fn notify_write_stall(&self) {
        let _state = self.write_stall.state.lock();
        self.write_stall.wake.notify_all();
    }

    /// Stop every stalled writer and reject later writes.
    pub(crate) fn shutdown_write_stall(&self) {
        self.write_stall.shutdown.store(true, Ordering::SeqCst);
        self.notify_write_stall();
    }

    /// Block (or fail) until the write watermarks permit another write.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorkvError::WriteStalled`] in non-blocking mode when a
    /// stop watermark is exceeded. Returns
    /// [`crate::StorkvError::InvalidOperation`] after the database shuts down.
    pub(crate) fn wait_for_write_capacity(
        &self,
        scope_ids: &[u32],
        options: WriteOptions,
    ) -> crate::Result<()> {
        let started = Instant::now();
        loop {
            if self.write_stall.shutdown.load(Ordering::SeqCst) {
                return Err(shutdown_error());
            }
            let l0 = self.max_l0_count(scope_ids);
            let wal_bytes = self.unflushed_wal_bytes.load(Ordering::Acquire);
            let l0_stop = self.options.l0_stop_writes_trigger;
            let wal_limit = self.options.max_total_wal_bytes;
            let l0_blocked = l0_stop > 0 && l0 >= l0_stop;
            let wal_blocked = wal_limit > 0 && wal_bytes >= wal_limit;

            if l0_blocked || wal_blocked {
                if options.non_blocking {
                    return Err(crate::StorkvError::WriteStalled(stall_reason(
                        l0,
                        l0_stop,
                        wal_bytes,
                        wal_limit,
                        l0_blocked,
                        wal_blocked,
                    )));
                }
                if started.elapsed() >= STALL_RELIEF_INTERVAL {
                    self.relieve_backlog(wal_blocked, l0_blocked)?;
                }
                let mut state = self.write_stall.state.lock();
                if self.write_stall.shutdown.load(Ordering::SeqCst) {
                    return Err(shutdown_error());
                }
                self.write_stall
                    .wake
                    .wait_for(&mut state, STALL_POLL_INTERVAL);
                continue;
            }

            let slowdown = self.options.l0_slowdown_writes_trigger;
            if !options.non_blocking && slowdown > 0 && l0 >= slowdown {
                std::thread::sleep(slowdown_delay(l0, slowdown, l0_stop));
            }
            return Ok(());
        }
    }

    fn max_l0_count(&self, scope_ids: &[u32]) -> usize {
        let scopes = self.scopes.read();
        scope_ids
            .iter()
            .filter_map(|scope_id| scopes.get(scope_id))
            .map(|scope| scope.sst_levels.read().get(&0).map(Vec::len).unwrap_or(0))
            .max()
            .unwrap_or(0)
    }

    fn relieve_backlog(&self, flush_wal: bool, compact_l0: bool) -> crate::Result<()> {
        if let Some(scheduler) = self.compaction_scheduler.lock().as_ref() {
            scheduler.notify();
        }
        if flush_wal {
            let scopes: Vec<Arc<Scope<FS>>> = self.scopes.read().values().cloned().collect();
            for scope in &scopes {
                self.flush_scope_memtable(scope)?;
            }
        }
        let scheduler_active = self.compaction_scheduler.lock().is_some();
        if compact_l0 && !scheduler_active {
            let trigger = self.options.l0_compaction_trigger.max(1);
            let max = self.options.max_background_compactions.max(1);
            self.run_due_compactions(trigger, max, None)?;
        }
        Ok(())
    }
}

fn shutdown_error() -> crate::StorkvError {
    crate::StorkvError::InvalidOperation("database is shutting down".into())
}

fn stall_reason(
    l0: usize,
    l0_stop: usize,
    wal_bytes: u64,
    wal_limit: u64,
    l0_blocked: bool,
    wal_blocked: bool,
) -> String {
    match (l0_blocked, wal_blocked) {
        (true, true) => format!(
            "L0 files {l0} reached {l0_stop} and un-flushed WAL bytes {wal_bytes} reached {wal_limit}"
        ),
        (true, false) => format!("L0 files {l0} reached the stop trigger of {l0_stop}"),
        _ => format!("un-flushed WAL bytes {wal_bytes} reached the limit of {wal_limit}"),
    }
}

fn slowdown_delay(l0: usize, slowdown: usize, stop: usize) -> Duration {
    let span = stop.saturating_sub(slowdown).max(1);
    let position = l0.saturating_sub(slowdown).saturating_add(1).min(span);
    let millis = MAX_WRITE_SLOWDOWN_DELAY_MS * position as u64 / span as u64;
    Duration::from_millis(millis.max(1))
}
