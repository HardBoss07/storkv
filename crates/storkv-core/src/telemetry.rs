use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Telemetry {
    pub writes_total: AtomicU64,
    pub reads_total: AtomicU64,
    pub write_conflicts_total: AtomicU64,
    pub compactions_total: AtomicU64,
    pub fsyncs_total: AtomicU64,
}

impl Telemetry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_write(&self) {
        self.writes_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_read(&self) {
        self.reads_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_write_conflict(&self) {
        self.write_conflicts_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_compaction(&self) {
        self.compactions_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_fsync(&self) {
        self.fsyncs_total.fetch_add(1, Ordering::Relaxed);
    }
}
