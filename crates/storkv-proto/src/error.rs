use bytes::{Buf, BufMut, BytesMut};
use thiserror::Error;

use crate::frame::{get_bytes, get_str, get_u32_le, put_bytes, put_str};

/// Wire-level error codes shared by clients and servers.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorKvWireError {
    #[error("Database locked: {0}")]
    DatabaseLocked(String),
    #[error("Storage corruption: {0}")]
    Corruption(String),
    #[error("Write conflict in scope {scope_id}")]
    WriteConflict { scope_id: u32, key: Vec<u8> },
    #[error("Scope not found: {0}")]
    ScopeNotFound(String),
    #[error("Scope ID not found: {0}")]
    ScopeIdNotFound(u32),
    #[error("Scope already exists: {0}")]
    ScopeAlreadyExists(String),
    #[error("Scope dropped: {0}")]
    ScopeDropped(u32),
    #[error("Snapshot not found: {0}")]
    SnapshotNotFound(String),
    #[error("Key exceeds maximum length: {actual} > {max}")]
    KeyTooLarge { max: usize, actual: usize },
    #[error("Value exceeds maximum length: {actual} > {max}")]
    ValueTooLarge { max: usize, actual: usize },
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
    #[error("IO error: {0}")]
    Io(String),
    #[error("Protocol error: {0}")]
    Protocol(String),
}

pub(crate) fn encode_error(destination: &mut BytesMut, error: &StorKvWireError) {
    match error {
        StorKvWireError::DatabaseLocked(message) => {
            destination.put_u8(1);
            put_str(destination, message);
        }
        StorKvWireError::Corruption(message) => {
            destination.put_u8(2);
            put_str(destination, message);
        }
        StorKvWireError::WriteConflict { scope_id, key } => {
            destination.put_u8(3);
            destination.put_u32_le(*scope_id);
            put_bytes(destination, key);
        }
        StorKvWireError::ScopeNotFound(name) => {
            destination.put_u8(4);
            put_str(destination, name);
        }
        StorKvWireError::ScopeIdNotFound(scope_id) => {
            destination.put_u8(5);
            destination.put_u32_le(*scope_id);
        }
        StorKvWireError::ScopeAlreadyExists(name) => {
            destination.put_u8(6);
            put_str(destination, name);
        }
        StorKvWireError::ScopeDropped(scope_id) => {
            destination.put_u8(7);
            destination.put_u32_le(*scope_id);
        }
        StorKvWireError::SnapshotNotFound(name) => {
            destination.put_u8(8);
            put_str(destination, name);
        }
        StorKvWireError::KeyTooLarge { max, actual } => {
            destination.put_u8(9);
            destination.put_u32_le(*max as u32);
            destination.put_u32_le(*actual as u32);
        }
        StorKvWireError::ValueTooLarge { max, actual } => {
            destination.put_u8(10);
            destination.put_u32_le(*max as u32);
            destination.put_u32_le(*actual as u32);
        }
        StorKvWireError::InvalidOperation(message) => {
            destination.put_u8(11);
            put_str(destination, message);
        }
        StorKvWireError::Io(message) => {
            destination.put_u8(12);
            put_str(destination, message);
        }
        StorKvWireError::Protocol(message) => {
            destination.put_u8(13);
            put_str(destination, message);
        }
    }
}

pub(crate) fn decode_error(source: &mut BytesMut) -> Result<StorKvWireError, StorKvWireError> {
    if source.is_empty() {
        return Err(StorKvWireError::Protocol("Empty error payload".into()));
    }
    let error_code = source.get_u8();
    match error_code {
        1 => Ok(StorKvWireError::DatabaseLocked(get_str(source)?)),
        2 => Ok(StorKvWireError::Corruption(get_str(source)?)),
        3 => {
            let scope_id = get_u32_le(source)?;
            let key = get_bytes(source)?.to_vec();
            Ok(StorKvWireError::WriteConflict { scope_id, key })
        }
        4 => Ok(StorKvWireError::ScopeNotFound(get_str(source)?)),
        5 => Ok(StorKvWireError::ScopeIdNotFound(get_u32_le(source)?)),
        6 => Ok(StorKvWireError::ScopeAlreadyExists(get_str(source)?)),
        7 => Ok(StorKvWireError::ScopeDropped(get_u32_le(source)?)),
        8 => Ok(StorKvWireError::SnapshotNotFound(get_str(source)?)),
        9 => {
            let max = get_u32_le(source)? as usize;
            let actual = get_u32_le(source)? as usize;
            Ok(StorKvWireError::KeyTooLarge { max, actual })
        }
        10 => {
            let max = get_u32_le(source)? as usize;
            let actual = get_u32_le(source)? as usize;
            Ok(StorKvWireError::ValueTooLarge { max, actual })
        }
        11 => Ok(StorKvWireError::InvalidOperation(get_str(source)?)),
        12 => Ok(StorKvWireError::Io(get_str(source)?)),
        13 => Ok(StorKvWireError::Protocol(get_str(source)?)),
        unknown_code => Err(StorKvWireError::Protocol(format!(
            "Unknown error code: {}",
            unknown_code
        ))),
    }
}
