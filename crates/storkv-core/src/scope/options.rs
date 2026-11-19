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

/// Storage residency mode for one scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum StorageMode {
    /// Store every structure on disk. This is the default mode.
    #[default]
    Disk,
    /// Keep all data in RAM. Skip the WAL and the SSTables.
    InMemory,
    /// Keep memtables and SSTables in RAM and write mutations to the disk WAL.
    InMemoryPersistent,
}

/// Configuration of one scope.
#[derive(Debug, Clone)]
pub struct ScopeOptions {
    /// Prefix Bloom filter size in bytes. `None` disables the filter.
    pub prefix_bloom_len: Option<usize>,
    /// Durability policy for commits in the scope.
    pub durability: Durability,
    /// Retention policy for keys in the scope.
    pub retention: Retention,
    /// Storage residency mode for the scope.
    pub storage_mode: StorageMode,
}

impl Default for ScopeOptions {
    fn default() -> Self {
        Self {
            durability: Durability::StrictFsync,
            retention: Retention::Permanent,
            prefix_bloom_len: None,
            storage_mode: StorageMode::Disk,
        }
    }
}
