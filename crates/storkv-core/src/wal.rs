//! Write-ahead log: operation records, CRC32C/LZ4 batch framing, segmented
//! writers with rotation/sealing, and torn-tail recovery.

mod codec;
mod record;
mod recovery;
mod writer;

pub use codec::WAL_FRAME_HEADER_SIZE;
pub use record::{Durability, WalBatchFrame, WalOp};
pub use recovery::WalRecovery;
pub use writer::{
    MAX_WAL_SEGMENT_BYTES, WAL_HEADER_SIZE, WAL_MAGIC, WAL_SEAL_INTERVAL, WalManager, WalSegment,
};
