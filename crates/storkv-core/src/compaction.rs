//! Compaction policy dispatch plus leveled and FIFO implementations.

mod fifo;
mod leveled;
mod leveled_range;
mod range_filter;

use crate::db::DbInner;
use crate::error::Result;
use crate::manifest::VersionEdit;
use crate::options::DbOptions;
use crate::scope::{CompactionStyle, Retention, Scope};
use crate::types::{SequenceNumber, TimestampMs};
use crate::vfs::FileSystem;
use parking_lot::{Condvar, Mutex};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

/// Compaction policy dispatcher.
pub struct Compactor;

impl Compactor {
    /// Run Leveled or FIFO compaction on a scope respecting gc_safe_seq
    ///
    /// # Errors
    ///
    /// Returns an error when compaction cannot read or write a file.
    pub fn compact_scope<FS: FileSystem>(
        scope: &Arc<Scope<FS>>,
        gc_safe_seq: SequenceNumber,
        next_file_num: &AtomicU64,
        now_ms: TimestampMs,
        rate_limiter: Option<&RateLimiter>,
    ) -> Result<Option<VersionEdit>> {
        if scope.is_dropped() {
            return Ok(None);
        }

        match scope.options.retention {
            Retention::Ttl {
                compaction_style: CompactionStyle::Fifo,
                ..
            } => Self::compact_fifo(scope, now_ms),
            _ => Self::compact_leveled(scope, gc_safe_seq, next_file_num, now_ms, rate_limiter),
        }
    }
}

/// Token-bucket limiter for compaction SSTable writes.
#[derive(Clone)]
pub struct RateLimiter {
    bucket: Arc<Mutex<Bucket>>,
    bytes_per_sec: f64,
}

struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    /// Create a limiter that allows `bytes_per_sec` bytes each second.
    pub fn new(bytes_per_sec: u64) -> Self {
        let rate = bytes_per_sec.max(1) as f64;
        Self {
            bucket: Arc::new(Mutex::new(Bucket {
                tokens: rate,
                last_refill: Instant::now(),
            })),
            bytes_per_sec: rate,
        }
    }

    /// Consume `bytes` tokens, sleeping until the bucket refills when needed.
    ///
    /// The bucket may hold a negative balance so concurrent callers share one
    /// debt and cannot burst more than `bytes_per_sec`.
    pub fn throttle(&self, bytes: usize) {
        let bytes = bytes as f64;
        let sleep_for = {
            let mut bucket = self.bucket.lock();
            let elapsed = bucket.last_refill.elapsed().as_secs_f64();
            bucket.last_refill = Instant::now();
            bucket.tokens = (bucket.tokens + elapsed * self.bytes_per_sec).min(self.bytes_per_sec);
            bucket.tokens -= bytes;
            if bucket.tokens >= 0.0 {
                return;
            }
            Duration::from_secs_f64(-bucket.tokens / self.bytes_per_sec)
        };
        std::thread::sleep(sleep_for);
    }
}

struct SignalState {
    pending: bool,
    shutdown: bool,
}

struct SchedulerSignal {
    state: Mutex<SignalState>,
    wake: Condvar,
}

/// Autonomous background compaction scheduler.
///
/// A dedicated thread wakes when a MemTable flush notifies it. It compacts at
/// most `max_background_compactions` scopes per wake, and only when a scope has
/// at least `l0_compaction_trigger` L0 SSTables. The optional rate limiter paces
/// the new SSTable writes. The thread stops on [`Db::close`](crate::Db) or drop.
pub struct CompactionScheduler<FS: FileSystem> {
    signal: Arc<SchedulerSignal>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    _filesystem: std::marker::PhantomData<fn() -> FS>,
}

impl<FS: FileSystem + 'static> CompactionScheduler<FS> {
    /// Start the scheduler for a database.
    ///
    /// # Errors
    ///
    /// Returns an error when the worker thread cannot start.
    pub fn start(options: &DbOptions, inner: &Arc<DbInner<FS>>) -> crate::Result<Self> {
        let signal = Arc::new(SchedulerSignal {
            state: Mutex::new(SignalState {
                pending: false,
                shutdown: false,
            }),
            wake: Condvar::new(),
        });
        let worker_signal = signal.clone();
        let weak_inner = Arc::downgrade(inner);
        let trigger = options.l0_compaction_trigger;
        let max_compactions = options.max_background_compactions.max(1);
        let limiter = options
            .compaction_rate_limit_bytes_per_sec
            .map(RateLimiter::new);

        let worker = std::thread::Builder::new()
            .name("storkv-compaction".into())
            .spawn(move || {
                loop {
                    {
                        let mut state = worker_signal.state.lock();
                        while !state.pending && !state.shutdown {
                            worker_signal.wake.wait(&mut state);
                        }
                        if state.shutdown {
                            break;
                        }
                        state.pending = false;
                    }
                    let Some(inner) = weak_inner.upgrade() else {
                        break;
                    };
                    let compacted =
                        inner.run_due_compactions(trigger, max_compactions, limiter.as_ref());
                    if compacted.unwrap_or(0) >= max_compactions {
                        worker_signal.state.lock().pending = true;
                    }
                }
            })
            .map_err(|error| crate::StorkvError::InvalidOperation(error.to_string()))?;

        Ok(Self {
            signal,
            worker: Mutex::new(Some(worker)),
            _filesystem: std::marker::PhantomData,
        })
    }
}

impl<FS: FileSystem> CompactionScheduler<FS> {
    /// Wake the worker after a MemTable flush.
    pub fn notify(&self) {
        self.signal.state.lock().pending = true;
        self.signal.wake.notify_one();
    }

    /// Stop the worker and wait for it to finish.
    pub fn shutdown(&self) {
        let mut state = self.signal.state.lock();
        state.pending = false;
        state.shutdown = true;
        self.signal.wake.notify_all();
        drop(state);
        if let Some(worker) = self.worker.lock().take() {
            let _ = worker.join();
        }
    }
}

impl<FS: FileSystem> Drop for CompactionScheduler<FS> {
    fn drop(&mut self) {
        self.shutdown();
    }
}
