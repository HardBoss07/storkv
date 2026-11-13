use crate::error::Result;
use bytes::Bytes;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
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
            if base.len() == 8 {
                i64::from_le_bytes(base.try_into().unwrap())
            } else if let Ok(s) = std::str::from_utf8(base) {
                s.parse::<i64>().unwrap_or(0)
            } else {
                0
            }
        } else {
            0
        };

        for op in operands {
            let val = if op.len() == 8 {
                i64::from_le_bytes(op.as_ref().try_into().unwrap())
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
        let v1 = if left.len() == 8 {
            i64::from_le_bytes(left.try_into().unwrap())
        } else {
            std::str::from_utf8(left).ok()?.parse::<i64>().ok()?
        };
        let v2 = if right.len() == 8 {
            i64::from_le_bytes(right.try_into().unwrap())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_operator() {
        let op = DefaultMergeOperator;
        let operands = vec![Bytes::from("5"), Bytes::from("10")];
        let merged = op.full_merge(b"cnt", Some(b"100"), &operands).unwrap();
        assert_eq!(merged.as_ref(), b"115");

        let p = op.partial_merge(b"cnt", b"10", b"20").unwrap();
        assert_eq!(p.as_ref(), b"30");
    }

    #[test]
    fn test_transaction_local_reads() {
        let mut tx = TransactionContext::new(50);
        tx.put(1, Bytes::from_static(b"k1"), Bytes::from_static(b"v1"), 0);
        let mutation = tx.get_local(1, b"k1").unwrap();
        match mutation {
            TxMutation::Put { val, .. } => assert_eq!(val.as_ref(), b"v1"),
            _ => panic!("Expected Put"),
        }
    }
}
