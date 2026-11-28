//! Runtime counters and latency samples.
use std::sync::atomic::{AtomicU64, Ordering};

/// Lock-free runtime counters for one database.
#[derive(Default)]
pub struct Telemetry {
    /// Total write operations.
    pub writes_total: AtomicU64,
    /// Total read operations.
    pub reads_total: AtomicU64,
    /// Total optimistic-concurrency write conflicts.
    pub write_conflicts_total: AtomicU64,
    /// Total compaction runs.
    pub compactions_total: AtomicU64,
    /// Total bytes written by compaction.
    pub compaction_bytes_written_total: AtomicU64,
    /// Total WAL sync operations.
    pub fsyncs_total: AtomicU64,
    /// Total successful backups.
    pub backups_completed_total: AtomicU64,
    /// Total failed backups.
    pub backups_failed_total: AtomicU64,
    /// Unix epoch milliseconds of the last successful backup.
    pub last_backup_timestamp_ms: AtomicU64,
    /// Sequence number of the last successful backup.
    pub last_backup_seq: AtomicU64,
    /// Total WAL segments sent to the backup sink.
    pub wal_segments_shipped_total: AtomicU64,
    /// Total SSTable block cache hits.
    pub block_cache_hits_total: AtomicU64,
    /// Total SSTable block cache misses.
    pub block_cache_misses_total: AtomicU64,
}

const _: () = assert!(std::mem::size_of::<Telemetry>() == 13 * std::mem::size_of::<u64>());

impl Telemetry {
    /// Create a counter set with all values at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the write counter.
    pub fn record_write(&self) {
        self.writes_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the read counter.
    pub fn record_read(&self) {
        self.reads_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the write-conflict counter.
    pub fn record_write_conflict(&self) {
        self.write_conflicts_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the compaction counter.
    pub fn record_compaction(&self) {
        self.compactions_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Add bytes written by compaction to the counter.
    pub fn record_compaction_bytes_written(&self, bytes: u64) {
        self.compaction_bytes_written_total
            .fetch_add(bytes, Ordering::Relaxed);
    }

    /// Increment the fsync counter.
    pub fn record_fsync(&self) {
        self.fsyncs_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a successful backup with its time and sequence number.
    pub fn record_backup_completed(&self, timestamp_ms: u64, sequence: u64) {
        self.backups_completed_total.fetch_add(1, Ordering::Relaxed);
        self.last_backup_timestamp_ms
            .store(timestamp_ms, Ordering::Relaxed);
        self.last_backup_seq.store(sequence, Ordering::Relaxed);
    }

    /// Increment the failed-backup counter.
    pub fn record_backup_failed(&self) {
        self.backups_failed_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the shipped-WAL-segment counter.
    pub fn record_wal_segment_shipped(&self) {
        self.wal_segments_shipped_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the block cache hit counter.
    pub fn record_block_cache_hit(&self) {
        self.block_cache_hits_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the block cache miss counter.
    pub fn record_block_cache_miss(&self) {
        self.block_cache_misses_total
            .fetch_add(1, Ordering::Relaxed);
    }
}
