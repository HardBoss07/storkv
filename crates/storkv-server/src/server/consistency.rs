//! Read consistency for one connection.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

/// Mutable read consistency for one connection.
///
/// Each connection starts from the server default. A
/// `ConfigureConsistency` request changes only this value.
pub(crate) struct ConnectionConsistency {
    mode: AtomicU8,
    max_lag_ms: AtomicU64,
}

impl ConnectionConsistency {
    /// Create the state from a consistency level.
    pub(crate) fn new(consistency: storkv_proto::Consistency) -> Self {
        let (mode, max_lag_ms) = consistency.to_wire();
        Self {
            mode: AtomicU8::new(mode),
            max_lag_ms: AtomicU64::new(max_lag_ms),
        }
    }

    /// Return the current consistency level.
    pub(crate) fn get(&self) -> storkv_proto::Consistency {
        let mode = self.mode.load(Ordering::Relaxed);
        let max_lag_ms = self.max_lag_ms.load(Ordering::Relaxed);
        storkv_proto::Consistency::from_wire(mode, max_lag_ms)
            .unwrap_or(storkv_proto::Consistency::Strong)
    }

    /// Replace the consistency level.
    pub(crate) fn set(&self, consistency: storkv_proto::Consistency) {
        let (mode, max_lag_ms) = consistency.to_wire();
        self.mode.store(mode, Ordering::Relaxed);
        self.max_lag_ms.store(max_lag_ms, Ordering::Relaxed);
    }
}
