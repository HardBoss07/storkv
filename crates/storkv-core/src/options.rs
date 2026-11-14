use crate::scope::{DEFAULT_MAX_WAL_PIN_AGE, DEFAULT_MAX_WAL_PIN_BYTES};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DbOptions {
    pub max_wal_pin_bytes: usize,
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
