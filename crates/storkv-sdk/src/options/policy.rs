use std::time::Duration;

use storkv_core::{CompactionStyle, Durability, Retention};

/// Retry policy for closure-based OCC transactions.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Add random jitter to each backoff delay.
    pub jitter: bool,
    /// Maximum number of retries after the first attempt.
    pub max_retries: u32,
    /// Delay before the first retry.
    pub base_delay: Duration,
    /// Upper bound for the exponential backoff delay.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            jitter: true,
            max_retries: 5,
            base_delay: Duration::from_millis(2),
            max_delay: Duration::from_millis(250),
        }
    }
}

impl RetryPolicy {
    /// Create a policy that never retries.
    pub fn none() -> Self {
        Self {
            jitter: false,
            max_retries: 0,
            base_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(0),
        }
    }

    /// Set the maximum number of retries.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Set the first backoff delay.
    pub fn with_base_delay(mut self, base_delay: Duration) -> Self {
        self.base_delay = base_delay;
        self
    }

    /// Set the maximum backoff delay.
    pub fn with_max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = max_delay;
        self
    }

    /// Enable or disable jitter.
    pub fn with_jitter(mut self, jitter: bool) -> Self {
        self.jitter = jitter;
        self
    }

    pub(crate) fn delay(&self, attempt: u32) -> Duration {
        let factor = 1u32.checked_shl(attempt.min(16)).unwrap_or(u32::MAX);
        let base = self.base_delay.saturating_mul(factor);
        let capped = if self.max_delay.is_zero() {
            base
        } else {
            base.min(self.max_delay)
        };
        if !self.jitter || capped.is_zero() {
            return capped;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.subsec_nanos())
            .unwrap_or(0);
        let spread = capped.as_nanos() / 2;
        let offset = u128::from(nanos) % spread.max(1);
        Duration::from_nanos(u64::try_from(offset).unwrap_or(u64::MAX))
    }
}

/// Configuration for the optional SDK read cache.
///
/// The cache is disabled by default. When enabled, each scope keeps a bounded
/// LRU map of point reads and a watch subscription that invalidates entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheOptions {
    /// Enable or disable client-side caching.
    pub enabled: bool,
    /// Maximum number of cached keys for each scope.
    pub max_entries_per_scope: usize,
}

impl Default for CacheOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            max_entries_per_scope: 1024,
        }
    }
}

impl CacheOptions {
    /// Create disabled cache options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable the cache.
    pub fn enable(mut self) -> Self {
        self.enabled = true;
        self
    }

    /// Set the maximum number of cached keys for each scope.
    pub fn max_entries_per_scope(mut self, max_entries_per_scope: usize) -> Self {
        self.max_entries_per_scope = max_entries_per_scope;
        self
    }
}

/// Declarative definition of one scope.
#[derive(Debug, Clone)]
pub struct ScopeDefinition {
    /// Prefix Bloom filter size in bytes. `None` disables the filter.
    pub prefix_bloom_len: Option<usize>,
    /// Durability policy for commits in the scope.
    pub durability: Durability,
    /// Key retention policy for the scope.
    pub retention: Retention,
    /// Unique scope name.
    pub name: String,
}

impl ScopeDefinition {
    /// Create a permanent, strictly durable scope definition.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            durability: Durability::StrictFsync,
            prefix_bloom_len: None,
            retention: Retention::Permanent,
            name: name.into(),
        }
    }

    /// Set the durability policy.
    pub fn with_durability(mut self, durability: Durability) -> Self {
        self.durability = durability;
        self
    }

    /// Set the prefix Bloom filter size in bytes.
    pub fn with_prefix_bloom_len(mut self, prefix_bloom_len: usize) -> Self {
        self.prefix_bloom_len = Some(prefix_bloom_len);
        self
    }

    /// Replace the retention policy.
    pub fn with_retention(mut self, retention: Retention) -> Self {
        self.retention = retention;
        self
    }

    /// Expire keys after `default_ttl` with the given compaction style.
    pub fn with_ttl(mut self, default_ttl: Duration, compaction_style: CompactionStyle) -> Self {
        self.retention = Retention::Ttl {
            default_ttl,
            compaction_style,
        };
        self
    }

    /// Return the configured default TTL, if any.
    #[must_use]
    pub fn default_ttl(&self) -> Option<Duration> {
        match self.retention {
            Retention::Ttl { default_ttl, .. } => Some(default_ttl),
            Retention::Permanent => None,
            _ => None,
        }
    }

    /// Return the configured compaction style, if any.
    #[must_use]
    pub fn compaction_style(&self) -> Option<CompactionStyle> {
        match self.retention {
            Retention::Ttl {
                compaction_style, ..
            } => Some(compaction_style),
            Retention::Permanent => None,
            _ => None,
        }
    }
}
