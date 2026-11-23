//! Built-in merge operator for the typed operands in [`MergeOp`].
//!
//! Values are read as 8-byte little-endian integers or as ASCII integers.
//! Counter results are stored as ASCII decimal strings.

use bytes::Bytes;

use super::{MergeOp, MergeOperator, operand};
use crate::error::Result;

/// Built-in merge operator that implements every [`MergeOp`] variant.
#[derive(Default, Clone)]
pub struct DefaultMergeOperator;

impl MergeOperator for DefaultMergeOperator {
    fn full_merge(
        &self,
        _key: &[u8],
        existing_value: Option<&[u8]>,
        operands: &[Bytes],
    ) -> Result<Bytes> {
        let mut current = existing_value.map(Bytes::copy_from_slice);
        for operand in operands {
            current = Some(operand::apply_operand(current.as_deref(), operand));
        }
        Ok(current.unwrap_or_default())
    }

    fn partial_merge(&self, _key: &[u8], left: &[u8], right: &[u8]) -> Option<Bytes> {
        operand::combine(left, right)
    }
}

const _: () = assert!(std::mem::size_of::<MergeOp>() <= 48);
