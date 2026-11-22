//! Scope option wire types.

use std::time::Duration;

/// Scope creation options as they appear on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireScopeOptions {
    /// Prefix Bloom filter size in bytes. `None` disables the filter.
    pub prefix_bloom_len: Option<usize>,
    /// Durability policy for commits in the new scope.
    pub durability: WireDurability,
    /// Retention policy for keys in the new scope.
    pub retention: WireRetention,
    /// Minimum allowed time to live. `None` disables the limit.
    pub min_ttl: Option<Duration>,
    /// Maximum allowed time to live. `None` disables the limit.
    pub max_ttl: Option<Duration>,
    /// Permit writes without an expiry.
    pub allow_permanent: bool,
}

/// Durability policy for committed writes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireDurability {
    /// Sync the WAL on every commit before the commit returns.
    StrictFsync,
    /// Sync the WAL on a timer. The duration is the maximum unsynced window.
    Buffered {
        /// Maximum time between background syncs.
        sync_interval: Duration,
    },
}

/// Key retention policy for a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireRetention {
    /// Keep keys until an explicit delete arrives.
    Permanent,
    /// Expire keys after a default TTL and compact with the selected style.
    Ttl {
        /// Default lifetime for a key that a write does not override.
        default_ttl: Duration,
        /// Compaction strategy for the scope.
        compaction_style: WireCompactionStyle,
    },
}

/// Compaction strategy for one scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireCompactionStyle {
    /// Size-tiered levels L0 through L6.
    Leveled,
    /// First-in, first-out retention by age.
    Fifo,
}
