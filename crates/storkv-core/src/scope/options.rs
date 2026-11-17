use crate::wal::Durability;
use std::time::Duration;

/// Default maximum bytes of WAL that an active reader may pin.
pub const DEFAULT_MAX_WAL_PIN_BYTES: usize = 64 * 1024 * 1024; // 64 MiB
/// Default maximum time that an active reader may pin the WAL.
pub const DEFAULT_MAX_WAL_PIN_AGE: Duration = Duration::from_secs(60); // 60s

/// Key retention policy for a scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Retention {
    /// Keep keys until an explicit delete arrives.
    Permanent,
    /// Expire keys after a default TTL and compact with the selected style.
    Ttl {
        /// Default lifetime for a key that a write does not override.
        default_ttl: Duration,
        /// Compaction strategy for the scope.
        compaction_style: CompactionStyle,
    },
}

/// Compaction strategy for one scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompactionStyle {
    /// Size-tiered levels L0 through L6.
    Leveled,
    /// First-in, first-out retention by age.
    Fifo,
}

/// Configuration of one scope.
#[derive(Debug, Clone)]
pub struct ScopeOptions {
    /// Durability policy for commits in the scope.
    pub durability: Durability,
    /// Retention policy for keys in the scope.
    pub retention: Retention,
    /// Prefix Bloom filter size in bytes. `None` disables the filter.
    pub prefix_bloom_len: Option<usize>,
}

impl Default for ScopeOptions {
    fn default() -> Self {
        Self {
            durability: Durability::StrictFsync,
            retention: Retention::Permanent,
            prefix_bloom_len: None,
        }
    }
}
