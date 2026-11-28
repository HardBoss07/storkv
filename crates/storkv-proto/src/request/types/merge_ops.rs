//! Merge operand and transaction mutation wire types.

use bytes::Bytes;

/// Merge operand sent over the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireMergeOp {
    /// Add an integer delta to the current counter value.
    CounterAdd(i64),
    /// Append bytes to the current value.
    AppendBytes {
        /// Bytes to append.
        data: Bytes,
        /// Maximum length of the result, or `None` for no bound.
        max_len: Option<u64>,
    },
    /// Keep the larger of the current value and this integer.
    MaxI64(i64),
    /// Keep the smaller of the current value and this integer.
    MinI64(i64),
    /// Apply a bitwise OR to the current value as a `u64`.
    BitOr(u64),
    /// Apply a bitwise AND to the current value as a `u64`.
    BitAnd(u64),
    /// Send a caller-defined operand to the merge operator.
    Custom(Bytes),
}

/// Pending transaction mutation sent in a commit request.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireTxMutation {
    /// Store a value. `0` in `expires_at_ms` means no expiry.
    Put {
        /// Value bytes to store.
        val: Bytes,
        /// Absolute Unix epoch milliseconds, or `0` for no expiry.
        expires_at_ms: u64,
    },
    /// Delete one key.
    Delete,
    /// Apply a merge operand to one key.
    Merge {
        /// Operand bytes for the merge operator.
        operand: Bytes,
    },
    /// Delete the range from the mutation key up to, but not including, `end_key`.
    DeleteRange {
        /// Exclusive end key of the range.
        end_key: Bytes,
    },
}
