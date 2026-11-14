use crate::error::Result;
use bytes::Bytes;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MergeOp {
    CounterAdd(i64),
    Custom(Bytes),
}

pub trait MergeOperator: Send + Sync {
    fn full_merge(
        &self,
        key: &[u8],
        existing_value: Option<&[u8]>,
        operands: &[Bytes],
    ) -> Result<Bytes>;
    fn partial_merge(&self, key: &[u8], left: &[u8], right: &[u8]) -> Option<Bytes>;
}

/// Default built-in CounterAdd merge operator
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxMutation {
    Put { val: Bytes, expires_at_ms: u64 },
    Delete,
    Merge { operand: Bytes },
    DeleteRange { end_key: Bytes },
}

/// Transaction with Read-Your-Own-Writes and OCC ReadSet validation
#[derive(Clone)]
pub struct TransactionContext {
    pub read_seq: u64,
    pub read_set: HashSet<(u32, Bytes)>,
    pub write_set: BTreeMap<(u32, Bytes), TxMutation>,
}

const _: () = assert!(std::mem::size_of::<TransactionContext>() <= 96);

impl TransactionContext {
    pub fn new(read_seq: u64) -> Self {
        Self {
            read_seq,
            read_set: HashSet::new(),
            write_set: BTreeMap::new(),
        }
    }

    pub fn record_read(&mut self, scope_id: u32, key: Bytes) {
        self.read_set.insert((scope_id, key));
    }

    pub fn put(&mut self, scope_id: u32, key: Bytes, val: Bytes, expires_at_ms: u64) {
        self.write_set
            .insert((scope_id, key), TxMutation::Put { val, expires_at_ms });
    }

    pub fn delete(&mut self, scope_id: u32, key: Bytes) {
        self.write_set.insert((scope_id, key), TxMutation::Delete);
    }

    pub fn merge(&mut self, scope_id: u32, key: Bytes, operand: Bytes) {
        self.write_set
            .insert((scope_id, key), TxMutation::Merge { operand });
    }

    pub fn get_local(&self, scope_id: u32, key: &[u8]) -> Option<&TxMutation> {
        self.write_set.get(&(scope_id, Bytes::copy_from_slice(key)))
    }
}
