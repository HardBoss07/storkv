//! Per-operation request counters and latency histograms.

use parking_lot::Mutex;

use storkv_proto::Request;

/// Latency histogram upper bounds in microseconds with their Prometheus labels.
pub(crate) const LATENCY_BUCKETS: [(&str, u64); 10] = [
    ("0.0001", 100),
    ("0.0005", 500),
    ("0.001", 1_000),
    ("0.005", 5_000),
    ("0.01", 10_000),
    ("0.05", 50_000),
    ("0.1", 100_000),
    ("0.5", 500_000),
    ("1", 1_000_000),
    ("5", 5_000_000),
];

/// One data operation class exposed as a metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpKind {
    /// Point read.
    Get,
    /// Point or merge write.
    Put,
    /// Conditional write.
    PutIf,
    /// Point or range delete.
    Delete,
    /// Range or prefix scan.
    Scan,
    /// Transaction commit.
    Txn,
}

const OP_COUNT: usize = 6;

impl OpKind {
    pub(crate) const ALL: [OpKind; OP_COUNT] = [
        OpKind::Get,
        OpKind::Put,
        OpKind::PutIf,
        OpKind::Delete,
        OpKind::Scan,
        OpKind::Txn,
    ];

    pub(crate) fn from_request(request: &Request) -> Option<Self> {
        match request {
            Request::Get { .. } => Some(OpKind::Get),
            Request::Put { .. } | Request::PutWithTtl { .. } | Request::Merge { .. } => {
                Some(OpKind::Put)
            }
            Request::PutIf { .. } => Some(OpKind::PutIf),
            Request::Delete { .. } | Request::DeleteRange { .. } => Some(OpKind::Delete),
            Request::Scan { .. } | Request::ScanPrefix { .. } => Some(OpKind::Scan),
            Request::TxCommit { .. } => Some(OpKind::Txn),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            OpKind::Get => "get",
            OpKind::Put => "put",
            OpKind::PutIf => "put_if",
            OpKind::Delete => "delete",
            OpKind::Scan => "scan",
            OpKind::Txn => "txn",
        }
    }

    fn index(self) -> usize {
        match self {
            OpKind::Get => 0,
            OpKind::Put => 1,
            OpKind::PutIf => 2,
            OpKind::Delete => 3,
            OpKind::Scan => 4,
            OpKind::Txn => 5,
        }
    }
}

#[derive(Default)]
struct OperationState {
    counts: [u64; OP_COUNT],
    buckets: [[u64; LATENCY_BUCKETS.len()]; OP_COUNT],
    sum_us: [u64; OP_COUNT],
}

/// Snapshot of one operation counter set for rendering.
pub(crate) struct OpSnapshot {
    pub(crate) kind: OpKind,
    pub(crate) count: u64,
    pub(crate) sum_us: u64,
    bucket_counts: [u64; LATENCY_BUCKETS.len()],
}

impl OpSnapshot {
    /// Return the cumulative count for one histogram bucket.
    pub(crate) fn bucket_count(&self, index: usize) -> u64 {
        self.bucket_counts.iter().take(index + 1).copied().sum()
    }
}

/// Counters for the six data operation classes.
#[derive(Default)]
pub(crate) struct OperationStats {
    state: Mutex<OperationState>,
}

impl OperationStats {
    pub(crate) fn record(&self, kind: OpKind, elapsed_us: u64) {
        let mut state = self.state.lock();
        let index = kind.index();
        state.counts[index] += 1;
        state.sum_us[index] = state.sum_us[index].saturating_add(elapsed_us);
        let bucket = LATENCY_BUCKETS
            .iter()
            .position(|(_, upper_us)| elapsed_us <= *upper_us);
        if let Some(bucket) = bucket {
            state.buckets[index][bucket] += 1;
        }
    }

    pub(crate) fn snapshot(&self) -> Vec<OpSnapshot> {
        let state = self.state.lock();
        OpKind::ALL
            .iter()
            .map(|kind| {
                let index = kind.index();
                OpSnapshot {
                    kind: *kind,
                    count: state.counts[index],
                    sum_us: state.sum_us[index],
                    bucket_counts: state.buckets[index],
                }
            })
            .collect()
    }
}
