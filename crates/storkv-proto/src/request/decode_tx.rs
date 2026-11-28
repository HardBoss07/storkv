//! Transaction commit request decoding.
//!
//! Read and write sets carry explicit counts. The decoder rejects counts that
//! cannot fit in the remaining payload before it reserves memory.

use bytes::{Bytes, BytesMut};

use super::types::{Request, WireTxMutation};
use crate::error::StorkvWireError;
use crate::frame::{get_bytes, get_u8, get_u32_le, get_u64_le};

pub(crate) fn decode_tx_commit_request(source: &mut BytesMut) -> Result<Request, StorkvWireError> {
    let read_seq = get_u64_le(source)?;
    let read_set = decode_read_set(source)?;
    let write_set = decode_write_set(source)?;
    Ok(Request::TxCommit {
        read_seq,
        read_set,
        write_set,
    })
}

fn decode_read_set(source: &mut BytesMut) -> Result<Vec<(u32, Bytes)>, StorkvWireError> {
    let read_count = get_u32_le(source)? as usize;
    // Each read-set entry needs at least a u32 scope id and a u32
    // length prefix. Reject impossible counts before reserving memory.
    if read_count > source.len() / 8 {
        return Err(StorkvWireError::Protocol(
            "Read-set count exceeds available payload".into(),
        ));
    }
    let mut read_set = Vec::with_capacity(read_count);
    for _ in 0..read_count {
        let scope_id = get_u32_le(source)?;
        let user_key = get_bytes(source)?;
        read_set.push((scope_id, user_key));
    }
    Ok(read_set)
}

type TxWriteSet = Vec<((u32, Bytes), WireTxMutation)>;

fn decode_write_set(source: &mut BytesMut) -> Result<TxWriteSet, StorkvWireError> {
    let write_count = get_u32_le(source)? as usize;
    if write_count > source.len() / 9 {
        return Err(StorkvWireError::Protocol(
            "Write-set count exceeds available payload".into(),
        ));
    }
    let mut write_set = Vec::with_capacity(write_count);
    for _ in 0..write_count {
        let scope_id = get_u32_le(source)?;
        let user_key = get_bytes(source)?;
        let mutation = decode_tx_mutation(source)?;
        write_set.push(((scope_id, user_key), mutation));
    }
    Ok(write_set)
}

fn decode_tx_mutation(source: &mut BytesMut) -> Result<WireTxMutation, StorkvWireError> {
    let mutation_type = get_u8(source)?;
    match mutation_type {
        1 => {
            let entry_value = get_bytes(source)?;
            let expires_at_ms = get_u64_le(source)?;
            Ok(WireTxMutation::Put {
                val: entry_value,
                expires_at_ms,
            })
        }
        2 => Ok(WireTxMutation::Delete),
        3 => Ok(WireTxMutation::Merge {
            operand: get_bytes(source)?,
        }),
        4 => Ok(WireTxMutation::DeleteRange {
            end_key: get_bytes(source)?,
        }),
        _ => Err(StorkvWireError::Protocol("Invalid mutation type".into())),
    }
}
