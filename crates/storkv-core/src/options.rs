use crate::scope::{DEFAULT_MAX_WAL_PIN_AGE, DEFAULT_MAX_WAL_PIN_BYTES};
use std::time::Duration;

/// Database-wide options shared by all scopes.
#[derive(Debug, Clone)]
pub struct DbOptions {
    /// Maximum bytes of WAL that an active reader may pin.
    pub max_wal_pin_bytes: usize,
    /// Maximum time that an active reader may pin the WAL.
    pub max_wal_pin_age: Duration,
}

impl Default for DbOptions {
    fn default() -> Self {
        Self {
            max_wal_pin_bytes: DEFAULT_MAX_WAL_PIN_BYTES,
            max_wal_pin_age: DEFAULT_MAX_WAL_PIN_AGE,
        }
    }
}
