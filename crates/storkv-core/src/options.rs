//! Database-wide options, limits, and defaults.
use crate::scope::{DEFAULT_MAX_WAL_PIN_AGE, DEFAULT_MAX_WAL_PIN_BYTES};
use std::time::Duration;

/// Default SSTable block cache budget in bytes (64 MiB).
pub const DEFAULT_BLOCK_CACHE_BYTES: usize = 64 * 1024 * 1024;
/// Default maximum un-flushed WAL bytes (256 MiB).
pub const DEFAULT_MAX_TOTAL_WAL_BYTES: u64 = 256 * 1024 * 1024;
/// Default L0 SSTable count that starts progressive write slowdown.
pub const DEFAULT_L0_SLOWDOWN_WRITES_TRIGGER: usize = 8;
/// Default L0 SSTable count that stops foreground writes.
pub const DEFAULT_L0_STOP_WRITES_TRIGGER: usize = 12;

/// Per-write behaviour options.
#[derive(Debug, Clone, Copy, Default)]
pub struct WriteOptions {
    /// Return `crate::StorkvError::WriteStalled` instead of blocking when a
    /// stop watermark is exceeded.
    pub non_blocking: bool,
}

/// Database-wide options shared by all scopes.
#[derive(Debug, Clone)]
pub struct DbOptions {
    /// Maximum bytes of WAL that an active reader may pin.
    pub max_wal_pin_bytes: usize,
    /// Maximum time that an active reader may pin the WAL.
    pub max_wal_pin_age: Duration,
    /// Enable autonomous background compaction.
    pub enable_background_compaction: bool,
    /// L0 SSTable count trigger that wakes the compaction scheduler.
    pub l0_compaction_trigger: usize,
    /// Maximum number of concurrent background compactions.
    pub max_background_compactions: usize,
    /// Optional rate limit for SSTable writes during compaction (bytes/sec).
    /// `None` means no rate limiting.
    pub compaction_rate_limit_bytes_per_sec: Option<u64>,
    /// Total byte budget of the shared SSTable block cache.
    pub block_cache_bytes: usize,
    /// Maximum total un-flushed WAL bytes. `0` disables the watermark.
    pub max_total_wal_bytes: u64,
    /// L0 SSTable count that starts progressive write slowdown.
    ///
    /// `0` disables the slowdown delay.
    pub l0_slowdown_writes_trigger: usize,
    /// L0 SSTable count that stops foreground writes. `0` disables the stop.
    pub l0_stop_writes_trigger: usize,
}

impl Default for DbOptions {
    fn default() -> Self {
        Self {
            max_wal_pin_bytes: DEFAULT_MAX_WAL_PIN_BYTES,
            max_wal_pin_age: DEFAULT_MAX_WAL_PIN_AGE,
            enable_background_compaction: false,
            l0_compaction_trigger: 4,
            max_background_compactions: 2,
            compaction_rate_limit_bytes_per_sec: None,
            block_cache_bytes: DEFAULT_BLOCK_CACHE_BYTES,
            max_total_wal_bytes: DEFAULT_MAX_TOTAL_WAL_BYTES,
            l0_slowdown_writes_trigger: DEFAULT_L0_SLOWDOWN_WRITES_TRIGGER,
            l0_stop_writes_trigger: DEFAULT_L0_STOP_WRITES_TRIGGER,
        }
    }
}
