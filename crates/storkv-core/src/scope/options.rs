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
    /// Minimum allowed time to live. `None` disables the limit.
    pub min_ttl: Option<Duration>,
    /// Maximum allowed time to live. `None` disables the limit.
    pub max_ttl: Option<Duration>,
    /// Permit writes without an expiry.
    pub allow_permanent: bool,
}

impl Default for ScopeOptions {
    fn default() -> Self {
        Self {
            durability: Durability::StrictFsync,
            retention: Retention::Permanent,
            prefix_bloom_len: None,
            storage_mode: StorageMode::Disk,
            min_ttl: None,
            max_ttl: None,
            allow_permanent: true,
        }
    }
}

impl ScopeOptions {
    /// Check that a write expiry obeys the scope limits.
    ///
    /// `0` means the write has no expiry. `now_ms` is the reference time.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorkvError::InvalidTtl`] when the scope forbids
    /// permanent writes or the lifetime is outside the configured limits.
    pub fn validate_expiry(&self, expires_at_ms: u64, now_ms: u64) -> crate::Result<()> {
        if expires_at_ms == 0 {
            if self.allow_permanent {
                return Ok(());
            }
            return Err(crate::StorkvError::InvalidTtl(
                "permanent writes are not allowed in this scope".into(),
            ));
        }
        let ttl_ms = expires_at_ms.saturating_sub(now_ms);
        if let Some(min_ttl) = self.min_ttl {
            if ttl_ms < min_ttl.as_millis() as u64 {
                return Err(crate::StorkvError::InvalidTtl(format!(
                    "time to live of {ttl_ms} ms is below the minimum of {} ms",
                    min_ttl.as_millis()
                )));
            }
        }
        if let Some(max_ttl) = self.max_ttl {
            if ttl_ms > max_ttl.as_millis() as u64 {
                return Err(crate::StorkvError::InvalidTtl(format!(
                    "time to live of {ttl_ms} ms exceeds the maximum of {} ms",
                    max_ttl.as_millis()
                )));
            }
        }
        Ok(())
    }
}
