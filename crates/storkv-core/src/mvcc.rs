use crate::error::Result;
use bytes::Bytes;
use std::collections::{BTreeMap, HashSet};

/// A merge operand supplied by a caller.
///
/// Merge writes combine operands with the current value at read time. The
/// engine stores the operand until compaction folds it into a single value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MergeOp {
    /// Add an integer delta to the current counter value.
    CounterAdd(i64),
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

/// Built-in merge operator that implements [`MergeOp::CounterAdd`].
///
/// Values are read as 8-byte little-endian integers or as ASCII integers. The
/// result is stored as an ASCII decimal string.
#[derive(Default, Clone)]
pub struct DefaultMergeOperator;

impl MergeOperator for DefaultMergeOperator {
    fn full_merge(
        &self,
        _key: &[u8],
        existing_value: Option<&[u8]>,
        operands: &[Bytes],
    ) -> Result<Bytes> {
        let mut total: i64 = if let Some(base) = existing_value {
            if let Ok(arr) = <[u8; 8]>::try_from(base) {
                i64::from_le_bytes(arr)
            } else if let Ok(s) = std::str::from_utf8(base) {
                s.parse::<i64>().unwrap_or(0)
            } else {
                0
            }
        } else {
            0
        };

        for op in operands {
            let val = if let Ok(arr) = <[u8; 8]>::try_from(op.as_ref()) {
                i64::from_le_bytes(arr)
            } else if let Ok(s) = std::str::from_utf8(op) {
                s.parse::<i64>().unwrap_or(0)
            } else {
                0
            };
            total = total.wrapping_add(val);
        }

        // Store back formatted as ASCII string or 8-byte LE
        Ok(Bytes::from(total.to_string()))
    }

    fn partial_merge(&self, _key: &[u8], left: &[u8], right: &[u8]) -> Option<Bytes> {
        let v1 = if let Ok(arr) = <[u8; 8]>::try_from(left) {
            i64::from_le_bytes(arr)
        } else {
            std::str::from_utf8(left).ok()?.parse::<i64>().ok()?
        };
        let v2 = if let Ok(arr) = <[u8; 8]>::try_from(right) {
            i64::from_le_bytes(arr)
        } else {
            std::str::from_utf8(right).ok()?.parse::<i64>().ok()?
        };
        let sum = v1.wrapping_add(v2);
        Some(Bytes::from(sum.to_string()))
    }
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

/// Transaction with Read-Your-Own-Writes and OCC ReadSet validation
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
