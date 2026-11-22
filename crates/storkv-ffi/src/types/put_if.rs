use bytes::Bytes;

use super::StorkvSlice;

/// Condition kind for [`crate::storkv_put_if`].
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorkvPutConditionKind {
    /// Write only when the key is absent or expired.
    Absent = 0,
    /// Write only when the key is present and not expired.
    Present = 1,
    /// Write only when the visible version has `expected_seq`.
    ExpectedSeq = 2,
    /// Write only when the visible value equals `expected_value`.
    ExpectedValue = 3,
}

/// Condition passed to [`crate::storkv_put_if`].
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StorkvPutCondition {
    /// Condition kind.
    pub kind: StorkvPutConditionKind,
    /// Expected sequence number. Used only by [`StorkvPutConditionKind::ExpectedSeq`].
    pub expected_seq: u64,
    /// Expected value bytes. Used only by [`StorkvPutConditionKind::ExpectedValue`].
    pub expected_value: StorkvSlice,
}

/// Outcome of [`crate::storkv_put_if`].
///
/// A `seq` of `0` on a mismatch means the key is absent.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct StorkvPutIfResult {
    /// `1` when the write was applied, `0` when the condition did not match.
    pub applied: u8,
    /// Sequence number of the committed write or of the visible version.
    pub seq: u64,
}

impl StorkvPutIfResult {
    /// An empty result with the condition not applied.
    pub const EMPTY: Self = Self { applied: 0, seq: 0 };
}

impl StorkvPutCondition {
    pub(crate) fn to_core(self) -> Option<storkv_sdk::PutCondition> {
        use storkv_sdk::PutCondition;
        Some(match self.kind {
            StorkvPutConditionKind::Absent => PutCondition::Absent,
            StorkvPutConditionKind::Present => PutCondition::Present,
            StorkvPutConditionKind::ExpectedSeq => PutCondition::ExpectedSeq(self.expected_seq),
            StorkvPutConditionKind::ExpectedValue => {
                let bytes = unsafe { self.expected_value.as_bytes() };
                PutCondition::ExpectedValue(Bytes::copy_from_slice(bytes))
            }
        })
    }
}
