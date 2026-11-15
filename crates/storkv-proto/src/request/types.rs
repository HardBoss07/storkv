use bytes::Bytes;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireScopeOptions {
    pub durability: WireDurability,
    pub retention: WireRetention,
    pub prefix_bloom_len: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireDurability {
    StrictFsync,
    Buffered { sync_interval: Duration },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireRetention {
    Permanent,
    Ttl {
        default_ttl: Duration,
        compaction_style: WireCompactionStyle,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireCompactionStyle {
    Leveled,
    Fifo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireMergeOp {
    CounterAdd(i64),
    Custom(Bytes),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireTxMutation {
    Put { val: Bytes, expires_at_ms: u64 },
    Delete,
    Merge { operand: Bytes },
    DeleteRange { end_key: Bytes },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Request {
    Ping,
    Get {
        scope_id: u32,
        key: Bytes,
    },
    Put {
        scope_id: u32,
        key: Bytes,
        val: Bytes,
    },
    PutWithTtl {
        scope_id: u32,
        key: Bytes,
        val: Bytes,
        ttl: Duration,
    },
    Delete {
        scope_id: u32,
        key: Bytes,
    },
    DeleteRange {
        scope_id: u32,
        start_key: Bytes,
        end_key: Bytes,
    },
    Merge {
        scope_id: u32,
        key: Bytes,
        op: WireMergeOp,
    },
    Scan {
        scope_id: u32,
        start_key: Option<Bytes>,
        end_key: Option<Bytes>,
    },
    ScanPrefix {
        scope_id: u32,
        prefix: Bytes,
    },
    TxBegin,
    TxCommit {
        read_seq: u64,
        read_set: Vec<(u32, Bytes)>,
        write_set: Vec<((u32, Bytes), WireTxMutation)>,
    },
    CreateScope {
        name: String,
        options: WireScopeOptions,
    },
    DropScope {
        name: String,
    },
    CreateSnapshot {
        name: String,
    },
    UnpinSnapshot {
        name: String,
    },
    Checkpoint {
        dest_dir: String,
    },
    Compact,
    WatchSubscribe,
    TelemetryQuery,
    HealthCheck,
    AdminStats,
}
