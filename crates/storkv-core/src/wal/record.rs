//! WAL operation and batch frame types.
//!
//! Operation encoding lives in the `codec` child module. The frame layout is
//! stable: CRC32C, frame length, batch sequence, op count, flags, and payload.

use bytes::Bytes;
use std::time::Duration;

mod codec;

/// Durability policy for committed writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Durability {
    /// Sync the WAL before the commit returns.
    StrictFsync,
    /// Sync the WAL on a timer.
    Buffered {
        /// Maximum time between background syncs.
        sync_interval: Duration,
    },
}

/// One logical write in a WAL batch.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WalOp {
    /// Store a value with an absolute expiry time.
    Put {
        /// Identifier of the target scope.
        scope_id: u32,
        /// Absolute Unix epoch milliseconds, or `0` for no expiry.
        expires_at_ms: u64,
        /// User key.
        key: Bytes,
        /// Value bytes.
        val: Bytes,
    },
    /// Delete one key.
    Delete {
        /// Identifier of the target scope.
        scope_id: u32,
        /// User key.
        key: Bytes,
    },
    /// Apply a merge operand.
    Merge {
        /// Identifier of the target scope.
        scope_id: u32,
        /// User key.
        key: Bytes,
        /// Operand bytes.
        val: Bytes,
    },
    /// Delete a half-open key range.
    DeleteRange {
        /// Identifier of the target scope.
        scope_id: u32,
        /// Inclusive start key.
        start_key: Bytes,
        /// Exclusive end key.
        end_key: Bytes,
    },
    /// Drop a whole scope.
    DropScope {
        /// Identifier of the scope to drop.
        scope_id: u32,
    },
}

/// WAL batch frame:
/// +----------+-----------+-----------+----------+-------+----------+--------------------+
/// | crc32c   | frame_len | batch_seq | op_count | flags | reserved | payload            |
/// | (u32 LE) | (u32 LE)  | (u64 LE)  | (u32 LE) | (u16) | (u16)    | ([u8; ...])        |
/// +----------+-----------+-----------+----------+-------+----------+--------------------+
/// One atomic WAL batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalBatchFrame {
    /// Monotonic batch sequence number.
    pub batch_seq: u64,
    /// Operations in commit order.
    pub ops: Vec<WalOp>,
}

impl WalBatchFrame {
    /// Build a batch from an ordered operation list.
    pub fn new(batch_seq: u64, ops: Vec<WalOp>) -> Self {
        Self { batch_seq, ops }
    }
}
