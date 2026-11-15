use bytes::Bytes;

use crate::error::StorKvWireError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireScopeHandle {
    pub scope_id: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireTelemetrySnapshot {
    pub writes_total: u64,
    pub reads_total: u64,
    pub write_conflicts_total: u64,
    pub compactions_total: u64,
    pub fsyncs_total: u64,
}

/// Per-level SSTable statistics for the admin dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireLevelStat {
    pub level: u8,
    pub file_count: u32,
    pub size_bytes: u64,
}

/// Per-scope activity statistics for the admin dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireScopeStat {
    pub scope_id: u32,
    pub name: String,
    pub writes_total: u64,
    pub reads_total: u64,
    pub sst_files: u32,
}

/// Rich admin statistics: L0..L6 SSTable distribution, per-scope metrics, and
/// the observed WAL commit (fsync) latency percentile.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireAdminStats {
    pub levels: Vec<WireLevelStat>,
    pub scopes: Vec<WireScopeStat>,
    pub fsync_latency_p99_us: u64,
    pub fsync_latency_samples: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireChangeEvent {
    Put {
        scope_id: u32,
        key: Bytes,
        value: Bytes,
        seq: u64,
    },
    Delete {
        scope_id: u32,
        key: Bytes,
        seq: u64,
    },
    DropScope {
        scope_id: u32,
        seq: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Response {
    Pong,
    Empty,
    TxStarted { read_seq: u64 },
    Value(Option<Bytes>),
    Scan(Vec<(Bytes, Bytes)>),
    Scope(WireScopeHandle),
    Telemetry(WireTelemetrySnapshot),
    HealthOk,
    WatchEvent(WireChangeEvent),
    AdminStats(WireAdminStats),
    Error(StorKvWireError),
}
