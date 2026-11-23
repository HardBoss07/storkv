use std::collections::{BTreeMap, HashSet};

use bytes::Bytes;

use crate::error::Result;

mod operand;
mod operator;

pub use operator::DefaultMergeOperator;

impl MergeOp {
    /// Encode this operation as stored operand bytes.
    pub(crate) fn to_operand_bytes(&self) -> Bytes {
        operand::encode(self)
    }
}

/// A merge operand supplied by a caller.
///
/// Merge writes combine operands with the current value at read time. The
/// engine stores the operand until compaction folds it into a single value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MergeOp {
    /// Add an integer delta to the current counter value.
    CounterAdd(i64),
    /// Append bytes to the current value.
    ///
    /// When `max_len` is `Some`, the result keeps only the newest bytes up to
    /// that length.
    AppendBytes {
        /// Bytes to append.
        data: Bytes,
        /// Maximum length of the result, or `None` for no bound.
        max_len: Option<usize>,
    },
    /// Keep the larger of the current value and this integer.
    MaxI64(i64),
    /// Keep the smaller of the current value and this integer.
    MinI64(i64),
    /// Apply a bitwise OR to the current value as a `u64`.
    BitOr(u64),
    /// Apply a bitwise AND to the current value as a `u64`.
    BitAnd(u64),
    /// Send a caller-defined operand to the configured merge operator.
    Custom(Bytes),
}

/// Combines merge operands with an existing value.
///
/// Implement this trait to change how merge writes are resolved. The default
/// implementation is [`DefaultMergeOperator`].
pub trait MergeOperator: Send + Sync {
    /// Combine every operand with the current value and return the new value.
    ///
    /// `existing_value` is `None` when no value exists yet. `operands` are in
    /// commit order.
    fn full_merge(
        &self,
        key: &[u8],
        existing_value: Option<&[u8]>,
        operands: &[Bytes],
    ) -> Result<Bytes>;

    /// Combine two adjacent values during compaction.
    ///
    /// Return `None` to skip the partial merge and defer to [`Self::full_merge`].
    fn partial_merge(&self, key: &[u8], left: &[u8], right: &[u8]) -> Option<Bytes>;
}

/// One pending mutation inside a transaction write set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxMutation {
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

/// Condition for an atomic single-key write.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PutCondition {
    /// Write only when the key is absent or expired.
    Absent,
    /// Write only when the key is present and not expired.
    Present,
    /// Write only when the visible version has this sequence number.
    ExpectedSeq(u64),
    /// Write only when the visible value equals these bytes.
    ExpectedValue(Bytes),
}

/// Outcome of an atomic conditional write.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PutIfResult {
    /// The condition matched. The key now has this sequence number.
    Applied {
        /// Sequence number of the committed write.
        seq: u64,
    },
    /// The condition did not match.
    Conflict {
        /// Sequence number of the visible version, or `None` when absent.
        current_seq: Option<u64>,
        /// Value of the visible version, or `None` when absent.
        current_value: Option<Bytes>,
    },
}

/// Transaction with Read-Your-Own-Writes and OCC ReadSet validation.
#[derive(Clone)]
pub struct TransactionContext {
    /// Sequence number that the transaction reads at.
    pub read_seq: u64,
    /// Keys that the transaction read, as `(scope_id, key)` pairs.
    pub read_set: HashSet<(u32, Bytes)>,
    /// Pending mutations keyed by `(scope_id, key)`.
    pub write_set: BTreeMap<(u32, Bytes), TxMutation>,
}

const _: () = assert!(std::mem::size_of::<TransactionContext>() <= 96);

impl TransactionContext {
    /// Create a context that reads at the given sequence number.
    pub fn new(read_seq: u64) -> Self {
        Self {
            read_seq,
            read_set: HashSet::new(),
            write_set: BTreeMap::new(),
        }
    }

    /// Add a key to the optimistic read set.
    pub fn record_read(&mut self, scope_id: u32, key: Bytes) {
        self.read_set.insert((scope_id, key));
    }

    /// Buffer a value write.
    pub fn put(&mut self, scope_id: u32, key: Bytes, val: Bytes, expires_at_ms: u64) {
        self.write_set
            .insert((scope_id, key), TxMutation::Put { val, expires_at_ms });
    }

    /// Buffer a delete.
    pub fn delete(&mut self, scope_id: u32, key: Bytes) {
        self.write_set.insert((scope_id, key), TxMutation::Delete);
    }

    /// Buffer a merge operand.
    pub fn merge(&mut self, scope_id: u32, key: Bytes, operand: Bytes) {
        self.write_set
            .insert((scope_id, key), TxMutation::Merge { operand });
    }

    /// Return the pending mutation for one key, if any.
    pub fn get_local(&self, scope_id: u32, key: &[u8]) -> Option<&TxMutation> {
        self.write_set.get(&(scope_id, Bytes::copy_from_slice(key)))
    }
}
