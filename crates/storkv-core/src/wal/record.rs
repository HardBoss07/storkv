use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::time::Duration;

use crate::error::{Result, StorkvError};
use crate::format::{MAX_KEY_LEN, MAX_VAL_LEN};

/// Opcode of a put operation.
const OPCODE_PUT: u8 = 0x01;
/// Opcode of a delete operation.
const OPCODE_DELETE: u8 = 0x02;
/// Opcode of a merge operation.
const OPCODE_MERGE: u8 = 0x03;
/// Opcode of a range delete operation.
const OPCODE_DELETE_RANGE: u8 = 0x04;
/// Opcode of a scope drop operation.
const OPCODE_DROP_SCOPE: u8 = 0x05;
/// Size of the fixed operation header: opcode, scope id, and expiry.
const WAL_OP_HEADER_SIZE: usize = 1 + 4 + 8;
/// Size of the length prefix of a sized byte field.
const SIZED_FIELD_LENGTH_SIZE: usize = 4;

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

impl WalOp {
    /// Return the one-byte opcode of the operation.
    pub fn opcode(&self) -> u8 {
        match self {
            WalOp::Put { .. } => OPCODE_PUT,
            WalOp::Delete { .. } => OPCODE_DELETE,
            WalOp::Merge { .. } => OPCODE_MERGE,
            WalOp::DeleteRange { .. } => OPCODE_DELETE_RANGE,
            WalOp::DropScope { .. } => OPCODE_DROP_SCOPE,
        }
    }

    /// Encode the operation into a WAL batch payload.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::KeyTooLarge`] or [`StorkvError::ValueTooLarge`]
    /// when a field exceeds the format limit.
    pub fn encode(&self, destination: &mut BytesMut) -> Result<()> {
        match self {
            WalOp::Put {
                scope_id,
                expires_at_ms,
                key,
                val,
            } => {
                validate_key(key)?;
                validate_value(val)?;
                encode_header(destination, OPCODE_PUT, *scope_id, *expires_at_ms);
                put_sized_bytes(destination, key);
                put_sized_bytes(destination, val);
            }
            WalOp::Delete { scope_id, key } => {
                validate_key(key)?;
                encode_header(destination, OPCODE_DELETE, *scope_id, 0);
                put_sized_bytes(destination, key);
                put_sized_bytes(destination, &[]);
            }
            WalOp::Merge { scope_id, key, val } => {
                validate_key(key)?;
                validate_value(val)?;
                encode_header(destination, OPCODE_MERGE, *scope_id, 0);
                put_sized_bytes(destination, key);
                put_sized_bytes(destination, val);
            }
            WalOp::DeleteRange {
                scope_id,
                start_key,
                end_key,
            } => {
                validate_key(start_key)?;
                validate_key(end_key)?;
                encode_header(destination, OPCODE_DELETE_RANGE, *scope_id, 0);
                put_sized_bytes(destination, start_key);
                put_sized_bytes(destination, end_key);
            }
            WalOp::DropScope { scope_id } => {
                encode_header(destination, OPCODE_DROP_SCOPE, *scope_id, 0);
                put_sized_bytes(destination, &[]);
                put_sized_bytes(destination, &[]);
            }
        }
        Ok(())
    }

    /// Decode one operation from a WAL batch payload.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::Corruption`] on a short header or unknown opcode.
    pub fn decode(source: &mut impl Buf) -> Result<Self> {
        if source.remaining() < WAL_OP_HEADER_SIZE {
            return Err(StorkvError::Corruption("Incomplete WalOp header".into()));
        }
        let opcode = source.get_u8();
        let scope_id = source.get_u32_le();
        let expires_at_ms = source.get_u64_le();
        let key = read_sized_bytes(source, "key")?;
        let value = read_sized_bytes(source, "val")?;

        match opcode {
            OPCODE_PUT => Ok(WalOp::Put {
                scope_id,
                expires_at_ms,
                key,
                val: value,
            }),
            OPCODE_DELETE => Ok(WalOp::Delete { scope_id, key }),
            OPCODE_MERGE => Ok(WalOp::Merge {
                scope_id,
                key,
                val: value,
            }),
            OPCODE_DELETE_RANGE => Ok(WalOp::DeleteRange {
                scope_id,
                start_key: key,
                end_key: value,
            }),
            OPCODE_DROP_SCOPE => Ok(WalOp::DropScope { scope_id }),
            _ => Err(StorkvError::Corruption(format!(
                "Unknown WalOp opcode: 0x{:02x}",
                opcode
            ))),
        }
    }
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

fn validate_key(key: &[u8]) -> Result<()> {
    if key.len() > MAX_KEY_LEN {
        return Err(StorkvError::KeyTooLarge {
            max: MAX_KEY_LEN,
            actual: key.len(),
        });
    }
    Ok(())
}

fn validate_value(value: &[u8]) -> Result<()> {
    if value.len() > MAX_VAL_LEN {
        return Err(StorkvError::ValueTooLarge {
            max: MAX_VAL_LEN,
            actual: value.len(),
        });
    }
    Ok(())
}

fn encode_header(destination: &mut BytesMut, opcode: u8, scope_id: u32, expires_at_ms: u64) {
    destination.put_u8(opcode);
    destination.put_u32_le(scope_id);
    destination.put_u64_le(expires_at_ms);
}

fn put_sized_bytes(destination: &mut BytesMut, field: &[u8]) {
    destination.put_u32_le(field.len() as u32);
    destination.extend_from_slice(field);
}

fn read_sized_bytes(source: &mut impl Buf, field: &str) -> Result<Bytes> {
    if source.remaining() < SIZED_FIELD_LENGTH_SIZE {
        return Err(StorkvError::Corruption(format!(
            "Incomplete WalOp {} length",
            field
        )));
    }
    let field_length = source.get_u32_le() as usize;
    if source.remaining() < field_length {
        return Err(StorkvError::Corruption(format!(
            "Incomplete WalOp {}",
            field
        )));
    }
    let mut field_bytes = vec![0u8; field_length];
    source.copy_to_slice(&mut field_bytes);
    Ok(Bytes::from(field_bytes))
}
