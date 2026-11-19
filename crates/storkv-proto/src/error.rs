use bytes::{Buf, BufMut, BytesMut};
use thiserror::Error;

use crate::frame::{get_bytes, get_str, get_u32_le, get_u64_le, put_bytes, put_str};

/// Wire-level error codes shared by clients and servers.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorkvWireError {
    /// Another process holds the database directory lock.
    #[error("Database locked: {0}")]
    DatabaseLocked(String),
    /// A checksum or structure check failed on stored data.
    #[error("Storage corruption: {0}")]
    Corruption(String),
    /// Optimistic concurrency validation found a changed read key.
    #[error("Write conflict in scope {scope_id}")]
    WriteConflict {
        /// Identifier of the scope that reported the conflict.
        scope_id: u32,
        /// User key that changed after the read.
        key: Vec<u8>,
    },
    /// The named scope does not exist.
    #[error("Scope not found: {0}")]
    ScopeNotFound(String),
    /// The scope identifier does not exist.
    #[error("Scope ID not found: {0}")]
    ScopeIdNotFound(u32),
    /// A scope with the requested name already exists.
    #[error("Scope already exists: {0}")]
    ScopeAlreadyExists(String),
    /// The scope was dropped after the request started.
    #[error("Scope dropped: {0}")]
    ScopeDropped(u32),
    /// The named snapshot does not exist.
    #[error("Snapshot not found: {0}")]
    SnapshotNotFound(String),
    /// A key is longer than the protocol limit.
    #[error("Key exceeds maximum length: {actual} > {max}")]
    KeyTooLarge {
        /// Maximum permitted key length in bytes.
        max: usize,
        /// Actual key length in bytes.
        actual: usize,
    },
    /// A value is longer than the protocol limit.
    #[error("Value exceeds maximum length: {actual} > {max}")]
    ValueTooLarge {
        /// Maximum permitted value length in bytes.
        max: usize,
        /// Actual value length in bytes.
        actual: usize,
    },
    /// The server rejected a semantically invalid request.
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
    /// An input or output operation failed.
    #[error("IO error: {0}")]
    Io(String),
    /// A frame was malformed or used an unsupported version.
    #[error("Protocol error: {0}")]
    Protocol(String),
    /// The requested watch cursor is older than the retained WAL history.
    #[error("Watch cursor {requested} expired (oldest available: {oldest_available})")]
    WatchCursorExpired {
        /// Cursor sequence requested by the subscriber.
        requested: u64,
        /// Oldest sequence still available for replay.
        oldest_available: u64,
    },
    /// Strict admin mode rejected an administrative request.
    #[error("Unauthorized: this request requires an admin connection")]
    Unauthorized,
    /// This node is a replica and cannot accept writes.
    #[error("This node is a replica and cannot accept writes")]
    NotPrimary,
    /// The replica lags behind the primary by more than the allowed maximum.
    #[error("Replica lag {lag_ms} ms exceeds the maximum of {max_lag_ms} ms")]
    StaleReplica {
        /// Observed replication lag in milliseconds.
        lag_ms: u64,
        /// Maximum permitted lag in milliseconds.
        max_lag_ms: u64,
    },
    /// A stream registry reached its subscriber limit.
    #[error("Too many subscribers: {count} active (maximum: {max})")]
    TooManySubscribers {
        /// Number of active subscribers.
        count: usize,
        /// Maximum number of subscribers.
        max: usize,
    },
    /// A stream replay would exceed the event or byte budget.
    #[error("Replay budget exceeded: {events} events and {bytes} bytes")]
    ReplayBudgetExceeded {
        /// Number of events in the replay request.
        events: usize,
        /// Total payload bytes in the replay request.
        bytes: usize,
    },
}

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
