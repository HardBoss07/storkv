use crate::scope::{DEFAULT_MAX_WAL_PIN_AGE, DEFAULT_MAX_WAL_PIN_BYTES};
use std::time::Duration;

/// Default SSTable block cache budget in bytes (64 MiB).
pub const DEFAULT_BLOCK_CACHE_BYTES: usize = 64 * 1024 * 1024;

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
        }
    }
}
