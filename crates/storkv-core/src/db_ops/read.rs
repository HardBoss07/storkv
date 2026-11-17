use crate::StorKvError;
use crate::db::DbInner;
use crate::format::ValueType;
use crate::types::{SequenceNumber, TimestampMs};
use crate::vfs::FileSystem;
use bytes::Bytes;
use std::sync::atomic::Ordering;

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Read one key from the newest visible state.
    ///
    /// # Errors
    ///
    /// Returns [`StorKvError::ScopeIdNotFound`] when the scope is unknown.
    pub fn get(&self, scope_id: u32, key: &[u8]) -> crate::Result<Option<Bytes>> {
        let read_seq = self.last_visible_seq.load(Ordering::Acquire);
        self.get_at(scope_id, key, SequenceNumber::new(read_seq))
    }

    /// Read one key at a fixed read sequence.
    ///
    /// # Errors
    ///
    /// Returns [`StorKvError::ScopeIdNotFound`] when the scope is unknown, or a
    /// storage read error.
    pub fn get_at(
        &self,
        scope_id: u32,
        key: &[u8],
        read_seq: SequenceNumber,
    ) -> crate::Result<Option<Bytes>> {
        self.telemetry.record_read();
        let now_ms = TimestampMs::now();

        let scope = self
            .scopes
            .read()
            .get(&scope_id)
            .cloned()
            .ok_or(StorKvError::ScopeIdNotFound(scope_id))?;

        let versions = scope.get_versions(key, read_seq, now_ms)?;
        if versions.is_empty() {
            return Ok(None);
        }

        let (base_value, operands) = split_versions(versions);
        if operands.is_empty() {
            Ok(base_value)
        } else {
            let merged = self
                .merge_operator
                .full_merge(key, base_value.as_deref(), &operands)?;
            Ok(Some(merged))
        }
    }
}

fn split_versions(versions: Vec<(ValueType, Bytes)>) -> (Option<Bytes>, Vec<Bytes>) {
    let mut operands = Vec::new();
    let mut base_value = None;

    for (value_type, value) in versions {
        match value_type {
            ValueType::MergeOperand => operands.push(value),
            ValueType::Value => {
                base_value = Some(value);
                break;
            }
            ValueType::Tombstone | ValueType::RangeTombstone => break,
        }
    }

    operands.reverse();
    (base_value, operands)
}
