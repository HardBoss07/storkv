use crate::wal::Durability;
use std::time::Duration;

pub const DEFAULT_MAX_WAL_PIN_BYTES: usize = 64 * 1024 * 1024; // 64 MiB
pub const DEFAULT_MAX_WAL_PIN_AGE: Duration = Duration::from_secs(60); // 60s

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Retention {
    Permanent,
    Ttl {
        default_ttl: Duration,
        compaction_style: CompactionStyle,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompactionStyle {
    Leveled,
    Fifo,
}

#[derive(Debug, Clone)]
pub struct ScopeOptions {
    pub durability: Durability,
    pub retention: Retention,
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
