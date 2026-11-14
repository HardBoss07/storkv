//! Compaction policy dispatch plus leveled and FIFO implementations.

mod fifo;
mod leveled;

use crate::error::Result;
use crate::manifest::VersionEdit;
use crate::scope::{CompactionStyle, Retention, Scope};
use crate::types::{SequenceNumber, TimestampMs};
use crate::vfs::FileSystem;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

pub struct Compactor;

impl Compactor {
    /// Run Leveled or FIFO compaction on a scope respecting gc_safe_seq
    pub fn compact_scope<FS: FileSystem>(
        scope: &Arc<Scope<FS>>,
        gc_safe_seq: SequenceNumber,
        next_file_num: &AtomicU64,
        now_ms: TimestampMs,
    ) -> Result<Option<VersionEdit>> {
        if scope.is_dropped() {
            return Ok(None);
        }

        match scope.options.retention {
            Retention::Ttl {
                compaction_style: CompactionStyle::Fifo,
                ..
            } => Self::compact_fifo(scope, now_ms),
            _ => Self::compact_leveled(scope, gc_safe_seq, next_file_num, now_ms),
        }
    }
}
