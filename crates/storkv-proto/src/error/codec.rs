//! Binary encoding and decoding for wire error codes.
//!
//! Each error writes one tag byte and then its variant fields. The tag values
//! are stable and appear in `FORMAT.md`.

use bytes::{Buf, BufMut, BytesMut};

use super::StorkvWireError;
use crate::frame::{get_bytes, get_str, get_u32_le, get_u64_le, put_bytes, put_str};

pub(crate) fn encode_error(destination: &mut BytesMut, error: &StorkvWireError) {
    match error {
        StorkvWireError::DatabaseLocked(message) => {
            destination.put_u8(1);
            put_str(destination, message);
        }
        StorkvWireError::Corruption(message) => {
            destination.put_u8(2);
            put_str(destination, message);
        }
        StorkvWireError::WriteConflict { scope_id, key } => {
            destination.put_u8(3);
            destination.put_u32_le(*scope_id);
            put_bytes(destination, key);
        }
        StorkvWireError::ScopeNotFound(name) => {
            destination.put_u8(4);
            put_str(destination, name);
        }
        StorkvWireError::ScopeIdNotFound(scope_id) => {
            destination.put_u8(5);
            destination.put_u32_le(*scope_id);
        }
        StorkvWireError::ScopeAlreadyExists(name) => {
            destination.put_u8(6);
            put_str(destination, name);
        }
        StorkvWireError::ScopeDropped(scope_id) => {
            destination.put_u8(7);
            destination.put_u32_le(*scope_id);
        }
        StorkvWireError::SnapshotNotFound(name) => {
            destination.put_u8(8);
            put_str(destination, name);
        }
        StorkvWireError::KeyTooLarge { max, actual } => {
            destination.put_u8(9);
            destination.put_u32_le(*max as u32);
            destination.put_u32_le(*actual as u32);
        }
        StorkvWireError::ValueTooLarge { max, actual } => {
            destination.put_u8(10);
            destination.put_u32_le(*max as u32);
            destination.put_u32_le(*actual as u32);
        }
        StorkvWireError::InvalidOperation(message) => {
            destination.put_u8(11);
            put_str(destination, message);
        }
        StorkvWireError::Io(message) => {
            destination.put_u8(12);
            put_str(destination, message);
        }
        StorkvWireError::Protocol(message) => {
            destination.put_u8(13);
            put_str(destination, message);
        }
        StorkvWireError::Unauthorized => {
            destination.put_u8(14);
        }
        StorkvWireError::WatchCursorExpired {
            requested,
            oldest_available,
        } => {
            destination.put_u8(15);
            destination.put_u64_le(*requested);
            destination.put_u64_le(*oldest_available);
        }
        StorkvWireError::NotPrimary => {
            destination.put_u8(16);
        }
        StorkvWireError::StaleReplica { lag_ms, max_lag_ms } => {
            destination.put_u8(17);
            destination.put_u64_le(*lag_ms);
            destination.put_u64_le(*max_lag_ms);
        }
        StorkvWireError::TooManySubscribers { count, max } => {
            destination.put_u8(18);
            destination.put_u32_le(*count as u32);
            destination.put_u32_le(*max as u32);
        }
        StorkvWireError::ReplayBudgetExceeded { events, bytes } => {
            destination.put_u8(19);
            destination.put_u32_le(*events as u32);
            destination.put_u32_le(*bytes as u32);
        }
    }
}

pub(crate) fn decode_error(source: &mut BytesMut) -> Result<StorkvWireError, StorkvWireError> {
    if source.is_empty() {
        return Err(StorkvWireError::Protocol("Empty error payload".into()));
    }
    let error_code = source.get_u8();
    match error_code {
        1 => Ok(StorkvWireError::DatabaseLocked(get_str(source)?)),
        2 => Ok(StorkvWireError::Corruption(get_str(source)?)),
        3 => {
            let scope_id = get_u32_le(source)?;
            let key = get_bytes(source)?.to_vec();
            Ok(StorkvWireError::WriteConflict { scope_id, key })
        }
        4 => Ok(StorkvWireError::ScopeNotFound(get_str(source)?)),
        5 => Ok(StorkvWireError::ScopeIdNotFound(get_u32_le(source)?)),
        6 => Ok(StorkvWireError::ScopeAlreadyExists(get_str(source)?)),
        7 => Ok(StorkvWireError::ScopeDropped(get_u32_le(source)?)),
        8 => Ok(StorkvWireError::SnapshotNotFound(get_str(source)?)),
        9 => {
            let max = get_u32_le(source)? as usize;
            let actual = get_u32_le(source)? as usize;
            Ok(StorkvWireError::KeyTooLarge { max, actual })
        }
        10 => {
            let max = get_u32_le(source)? as usize;
            let actual = get_u32_le(source)? as usize;
            Ok(StorkvWireError::ValueTooLarge { max, actual })
        }
        11 => Ok(StorkvWireError::InvalidOperation(get_str(source)?)),
        12 => Ok(StorkvWireError::Io(get_str(source)?)),
        13 => Ok(StorkvWireError::Protocol(get_str(source)?)),
        14 => Ok(StorkvWireError::Unauthorized),
        15 => Ok(StorkvWireError::WatchCursorExpired {
            requested: get_u64_le(source)?,
            oldest_available: get_u64_le(source)?,
        }),
        16 => Ok(StorkvWireError::NotPrimary),
        17 => Ok(StorkvWireError::StaleReplica {
            lag_ms: get_u64_le(source)?,
            max_lag_ms: get_u64_le(source)?,
        }),
        18 => Ok(StorkvWireError::TooManySubscribers {
            count: get_u32_le(source)? as usize,
            max: get_u32_le(source)? as usize,
        }),
        19 => Ok(StorkvWireError::ReplayBudgetExceeded {
            events: get_u32_le(source)? as usize,
            bytes: get_u32_le(source)? as usize,
        }),
        unknown_code => Err(StorkvWireError::Protocol(format!(
            "Unknown error code: {}",
            unknown_code
        ))),
    }
}
