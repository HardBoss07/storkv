//! Pool and retry configuration types for the C ABI.

use std::time::Duration;

/// Configuration for a pooled FFI connection.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StorkvPoolConfig {
    /// Probe idle connections before reuse.
    pub health_check: bool,
    /// Maximum number of simultaneous connections.
    pub max_connections: u32,
    /// Number of connections opened eagerly.
    pub min_connections: u32,
    /// Acquire timeout in milliseconds. Zero selects the SDK default.
    pub acquire_timeout_ms: u64,
    /// Idle timeout in milliseconds. Zero disables idle expiry.
    pub idle_timeout_ms: u64,
}

impl Default for StorkvPoolConfig {
    fn default() -> Self {
        Self {
            health_check: true,
            max_connections: 10,
            min_connections: 1,
            acquire_timeout_ms: 5_000,
            idle_timeout_ms: 300_000,
        }
    }
}

impl StorkvPoolConfig {
    pub(crate) fn to_options(self) -> storkv_sdk::StorkvPoolOptions {
        let mut options = storkv_sdk::StorkvPoolOptions::new()
            .health_check(self.health_check)
            .max_connections(self.max_connections)
            .min_connections(self.min_connections);
        if self.acquire_timeout_ms > 0 {
            options = options.acquire_timeout(Duration::from_millis(self.acquire_timeout_ms));
        }
        options = options.idle_timeout(if self.idle_timeout_ms == 0 {
            None
        } else {
            Some(Duration::from_millis(self.idle_timeout_ms))
        });
        options
    }
}

/// Retry configuration for the automatic transaction helper.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StorkvRetryConfig {
    /// Add random jitter to each backoff delay.
    pub jitter: bool,
    /// Maximum number of retries after the first attempt.
    pub max_retries: u32,
    /// Delay before the first retry, in milliseconds.
    pub base_delay_ms: u64,
    /// Maximum backoff delay, in milliseconds.
    pub max_delay_ms: u64,
}

impl Default for StorkvRetryConfig {
    fn default() -> Self {
        Self {
            jitter: true,
            max_retries: 5,
            base_delay_ms: 2,
            max_delay_ms: 250,
        }
    }
}

impl StorkvRetryConfig {
    pub(crate) fn to_policy(self) -> storkv_sdk::RetryPolicy {
        storkv_sdk::RetryPolicy::default()
            .with_jitter(self.jitter)
            .with_max_retries(self.max_retries)
            .with_base_delay(Duration::from_millis(self.base_delay_ms))
            .with_max_delay(Duration::from_millis(self.max_delay_ms))
    }
}
