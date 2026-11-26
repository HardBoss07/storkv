use std::collections::BTreeSet;
use std::time::Duration;

use serde::Deserialize;
use storkv_core::{CompactionStyle, Durability, Retention, ScopeOptions, StorageMode};

use super::file::ConfigError;

/// Default sync interval for a buffered scope, in milliseconds.
const DEFAULT_BUFFERED_SYNC_MS: u64 = 1000;

/// One `[[scopes]]` entry from a `storkv.toml` file.
///
/// `storkvd` creates or verifies every declared scope before it accepts client
/// traffic.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScopeConfig {
    /// Unique scope name. The name is required.
    pub name: String,
    /// Durability policy: `strict_fsync` or `buffered`.
    pub durability: Option<String>,
    /// Retention policy: `permanent` or `ttl`.
    pub retention: Option<String>,
    /// Default lifetime for a `ttl` scope, in milliseconds.
    pub default_ttl_ms: Option<u64>,
    /// Minimum allowed time to live, in milliseconds.
    pub min_ttl_ms: Option<u64>,
    /// Maximum allowed time to live, in milliseconds.
    pub max_ttl_ms: Option<u64>,
    /// Permit writes without an expiry. Defaults to `true`.
    pub allow_permanent: Option<bool>,
    /// Actively track TTL deadlines and emit expiry events.
    pub active_ttl_tracking: Option<bool>,
    /// Storage mode: `disk`, `in_memory`, or `in_memory_persistent`.
    pub storage_mode: Option<String>,
    /// Prefix Bloom filter size in bytes.
    pub prefix_bloom_len: Option<usize>,
}

impl ScopeConfig {
    /// Build engine scope options from this entry.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Scope`] when the name is empty, a policy name is
    /// unknown, a TTL value is zero, missing, or inconsistent, a Bloom filter
    /// size is zero, or a policy field is set for the wrong retention mode.
    pub fn to_options(&self) -> Result<ScopeOptions, ConfigError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(self.error("the scope name is empty"));
        }
        if self.min_ttl_ms == Some(0) {
            return Err(self.error("min_ttl_ms must be greater than zero"));
        }
        if self.max_ttl_ms == Some(0) {
            return Err(self.error("max_ttl_ms must be greater than zero"));
        }
        if self.prefix_bloom_len == Some(0) {
            return Err(self.error("prefix_bloom_len must be greater than zero"));
        }
        let retention = self.retention()?;
        let min_ttl = self.min_ttl_ms.map(Duration::from_millis);
        let max_ttl = self.max_ttl_ms.map(Duration::from_millis);
        if let (Some(min), Some(max)) = (min_ttl, max_ttl)
            && min > max
        {
            return Err(self.error("min_ttl_ms must not exceed max_ttl_ms"));
        }
        Ok(ScopeOptions {
            durability: self.durability()?,
            retention,
            prefix_bloom_len: self.prefix_bloom_len,
            storage_mode: self.storage_mode()?,
            min_ttl,
            max_ttl,
            allow_permanent: self.allow_permanent.unwrap_or(true),
            active_ttl_tracking: self.active_ttl_tracking.unwrap_or(false),
        })
    }

    fn durability(&self) -> Result<Durability, ConfigError> {
        match self.durability.as_deref().unwrap_or("strict_fsync") {
            "strict_fsync" => Ok(Durability::StrictFsync),
            "buffered" => Ok(Durability::Buffered {
                sync_interval: Duration::from_millis(DEFAULT_BUFFERED_SYNC_MS),
            }),
            other => Err(self.error(format!(
                "unknown durability `{other}`; expected strict_fsync or buffered"
            ))),
        }
    }

    fn retention(&self) -> Result<Retention, ConfigError> {
        match self.retention.as_deref().unwrap_or("permanent") {
            "permanent" => {
                if self.default_ttl_ms.is_some() {
                    return Err(self.error("default_ttl_ms requires retention = \"ttl\""));
                }
                Ok(Retention::Permanent)
            }
            "ttl" => {
                let default_ttl_ms =
                    self.default_ttl_ms
                        .filter(|value| *value > 0)
                        .ok_or_else(|| {
                            self.error("retention = \"ttl\" requires a positive default_ttl_ms")
                        })?;
                let default_ttl = Duration::from_millis(default_ttl_ms);
                if let Some(min) = self.min_ttl_ms
                    && default_ttl_ms < min
                {
                    return Err(self.error("default_ttl_ms must not be below min_ttl_ms"));
                }
                if let Some(max) = self.max_ttl_ms
                    && default_ttl_ms > max
                {
                    return Err(self.error("default_ttl_ms must not exceed max_ttl_ms"));
                }
                Ok(Retention::Ttl {
                    default_ttl,
                    compaction_style: CompactionStyle::Leveled,
                })
            }
            other => Err(self.error(format!(
                "unknown retention `{other}`; expected permanent or ttl"
            ))),
        }
    }

    fn storage_mode(&self) -> Result<StorageMode, ConfigError> {
        match self.storage_mode.as_deref().unwrap_or("disk") {
            "disk" => Ok(StorageMode::Disk),
            "in_memory" => Ok(StorageMode::InMemory),
            "in_memory_persistent" => Ok(StorageMode::InMemoryPersistent),
            other => Err(self.error(format!(
                "unknown storage_mode `{other}`; expected disk, in_memory, or in_memory_persistent"
            ))),
        }
    }

    fn error(&self, detail: impl Into<String>) -> ConfigError {
        ConfigError::Scope {
            name: self.name.clone(),
            detail: detail.into(),
        }
    }
}

/// Convert every declared scope to engine options.
///
/// # Errors
///
/// Returns [`ConfigError::Scope`] for an invalid entry, an empty name, or a
/// duplicate name.
pub(super) fn collect(scopes: &[ScopeConfig]) -> Result<Vec<(String, ScopeOptions)>, ConfigError> {
    let mut names = BTreeSet::new();
    let mut options = Vec::with_capacity(scopes.len());
    for scope in scopes {
        let name = scope.name.trim().to_string();
        let scope_options = scope.to_options()?;
        if !names.insert(name.clone()) {
            return Err(ConfigError::Scope {
                name,
                detail: "the scope name is declared more than once".into(),
            });
        }
        options.push((name, scope_options));
    }
    Ok(options)
}
