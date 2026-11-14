use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::time::Duration;

use crate::error::{Result, StorKvError};
use crate::format::{MAX_KEY_LEN, MAX_VAL_LEN};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Durability {
    StrictFsync,
    Buffered { sync_interval: Duration },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WalOp {
    Put {
        scope_id: u32,
        expires_at_ms: u64,
        key: Bytes,
        val: Bytes,
    },
    Delete {
        scope_id: u32,
        key: Bytes,
    },
    Merge {
        scope_id: u32,
        key: Bytes,
        val: Bytes,
    },
    DeleteRange {
        scope_id: u32,
        start_key: Bytes,
        end_key: Bytes,
    },
    DropScope {
        scope_id: u32,
    },
}

impl WalOp {
    pub fn opcode(&self) -> u8 {
        match self {
            WalOp::Put { .. } => 0x01,
            WalOp::Delete { .. } => 0x02,
            WalOp::Merge { .. } => 0x03,
            WalOp::DeleteRange { .. } => 0x04,
            WalOp::DropScope { .. } => 0x05,
        }
    }

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
                encode_header(destination, 0x01, *scope_id, *expires_at_ms);
                put_sized_bytes(destination, key);
                put_sized_bytes(destination, val);
            }
            WalOp::Delete { scope_id, key } => {
                validate_key(key)?;
                encode_header(destination, 0x02, *scope_id, 0);
                put_sized_bytes(destination, key);
                put_sized_bytes(destination, &[]);
            }
            WalOp::Merge { scope_id, key, val } => {
                validate_key(key)?;
                validate_value(val)?;
                encode_header(destination, 0x03, *scope_id, 0);
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
                encode_header(destination, 0x04, *scope_id, 0);
                put_sized_bytes(destination, start_key);
                put_sized_bytes(destination, end_key);
            }
            WalOp::DropScope { scope_id } => {
                encode_header(destination, 0x05, *scope_id, 0);
                put_sized_bytes(destination, &[]);
                put_sized_bytes(destination, &[]);
            }
        }
        Ok(())
    }

    pub fn decode(source: &mut impl Buf) -> Result<Self> {
        if source.remaining() < 17 {
            return Err(StorKvError::Corruption("Incomplete WalOp header".into()));
        }
        let opcode = source.get_u8();
        let scope_id = source.get_u32_le();
        let expires_at_ms = source.get_u64_le();
        let key = read_sized_bytes(source, "key")?;
        let value = read_sized_bytes(source, "val")?;

        match opcode {
            0x01 => Ok(WalOp::Put {
                scope_id,
                expires_at_ms,
                key,
                val: value,
            }),
            0x02 => Ok(WalOp::Delete { scope_id, key }),
            0x03 => Ok(WalOp::Merge {
                scope_id,
                key,
                val: value,
            }),
            0x04 => Ok(WalOp::DeleteRange {
                scope_id,
                start_key: key,
                end_key: value,
            }),
            0x05 => Ok(WalOp::DropScope { scope_id }),
            _ => Err(StorKvError::Corruption(format!(
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
pub struct WalBatchFrame {
    pub batch_seq: u64,
    pub ops: Vec<WalOp>,
}

impl WalBatchFrame {
    pub fn new(batch_seq: u64, ops: Vec<WalOp>) -> Self {
        Self { batch_seq, ops }
    }
}

fn validate_key(key: &[u8]) -> Result<()> {
    if key.len() > MAX_KEY_LEN {
        return Err(StorKvError::KeyTooLarge {
            max: MAX_KEY_LEN,
            actual: key.len(),
        });
    }
    Ok(())
}

fn validate_value(value: &[u8]) -> Result<()> {
    if value.len() > MAX_VAL_LEN {
        return Err(StorKvError::ValueTooLarge {
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
    if source.remaining() < 4 {
        return Err(StorKvError::Corruption(format!(
            "Incomplete WalOp {} length",
            field
        )));
    }
    let field_length = source.get_u32_le() as usize;
    if source.remaining() < field_length {
        return Err(StorKvError::Corruption(format!(
            "Incomplete WalOp {}",
            field
        )));
    }
    let mut field_bytes = vec![0u8; field_length];
    source.copy_to_slice(&mut field_bytes);
    Ok(Bytes::from(field_bytes))
}
