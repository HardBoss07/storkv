//! Binary encoding and application of typed merge operands.
//!
//! A counter operand stays an ASCII decimal, and a custom operand stays raw
//! bytes. Every other operand starts with [`TAG`]. Stored operands keep the
//! existing `MergeOperand` record type, so the on-disk format does not change.

use bytes::{BufMut, Bytes, BytesMut};

use super::MergeOp;

/// First byte of every typed operand. An ASCII integer never starts with it.
const TAG: u8 = 0xFF;

const APPEND_BYTES: u8 = 1;
const MAX_I64: u8 = 2;
const MIN_I64: u8 = 3;
const BIT_OR: u8 = 4;
const BIT_AND: u8 = 5;

/// Encode one merge operation as its stored operand bytes.
pub(super) fn encode(op: &MergeOp) -> Bytes {
    let mut buffer = BytesMut::new();
    match op {
        MergeOp::CounterAdd(delta) => buffer.put_slice(delta.to_string().as_bytes()),
        MergeOp::Custom(operand) => buffer.put_slice(operand),
        MergeOp::AppendBytes { data, max_len } => {
            buffer.put_u8(TAG);
            buffer.put_u8(APPEND_BYTES);
            buffer.put_u8(u8::from(max_len.is_some()));
            if let Some(max_len) = max_len {
                buffer.put_u64_le(*max_len as u64);
            }
            buffer.put_slice(data);
        }
        MergeOp::MaxI64(value) => buffer.put_slice(&signed(TAG, MAX_I64, *value)),
        MergeOp::MinI64(value) => buffer.put_slice(&signed(TAG, MIN_I64, *value)),
        MergeOp::BitOr(bits) => buffer.put_slice(&unsigned(TAG, BIT_OR, *bits)),
        MergeOp::BitAnd(bits) => buffer.put_slice(&unsigned(TAG, BIT_AND, *bits)),
    }
    buffer.freeze()
}

/// Decode a typed operand. `None` means the bytes use the legacy format.
pub(super) fn decode(operand: &[u8]) -> Option<MergeOp> {
    if operand.first() != Some(&TAG) || operand.len() < 2 {
        return None;
    }
    let mut source = &operand[2..];
    match operand[1] {
        APPEND_BYTES => {
            let (&has_bound, rest) = source.split_first()?;
            source = rest;
            let max_len = if has_bound == 0 {
                None
            } else {
                let bytes = <[u8; 8]>::try_from(source.get(..8)?).ok()?;
                source = &source[8..];
                Some(usize::try_from(u64::from_le_bytes(bytes)).ok()?)
            };
            Some(MergeOp::AppendBytes {
                data: Bytes::copy_from_slice(source),
                max_len,
            })
        }
        MAX_I64 => Some(MergeOp::MaxI64(read_i64(source)?)),
        MIN_I64 => Some(MergeOp::MinI64(read_i64(source)?)),
        BIT_OR => Some(MergeOp::BitOr(read_u64(source)?)),
        BIT_AND => Some(MergeOp::BitAnd(read_u64(source)?)),
        _ => None,
    }
}

/// Apply one stored operand to the current value.
pub(super) fn apply_operand(current: Option<&[u8]>, operand: &[u8]) -> Bytes {
    match decode(operand) {
        Some(op) => apply(current, &op),
        None => Bytes::from(
            parse_i64(current)
                .wrapping_add(parse_i64(Some(operand)))
                .to_string(),
        ),
    }
}

/// Apply one merge operation to the current value.
pub(super) fn apply(current: Option<&[u8]>, op: &MergeOp) -> Bytes {
    match op {
        MergeOp::CounterAdd(delta) => {
            let total = parse_i64(current).wrapping_add(*delta);
            Bytes::from(total.to_string())
        }
        MergeOp::Custom(operand) => operand.clone(),
        MergeOp::AppendBytes { data, max_len } => {
            let mut joined =
                BytesMut::with_capacity(current.map_or(0, <[u8]>::len).saturating_add(data.len()));
            if let Some(current) = current {
                joined.put_slice(current);
            }
            joined.put_slice(data);
            let start = max_len.map_or(0, |max_len| joined.len().saturating_sub(max_len));
            joined.split_off(start).freeze()
        }
        MergeOp::MaxI64(value) => {
            let next = current.map_or(*value, |current| parse_i64(Some(current)).max(*value));
            Bytes::from(next.to_string())
        }
        MergeOp::MinI64(value) => {
            let next = current.map_or(*value, |current| parse_i64(Some(current)).min(*value));
            Bytes::from(next.to_string())
        }
        MergeOp::BitOr(bits) => Bytes::from((parse_u64(current) | bits).to_string()),
        MergeOp::BitAnd(bits) => Bytes::from((parse_u64(current) & bits).to_string()),
    }
}

/// Fold two adjacent stored operands into one equivalent operand when associative.
///
/// Returns `None` when the operands cannot be folded without the base value.
pub(super) fn combine(left: &[u8], right: &[u8]) -> Option<Bytes> {
    match (decode(left), decode(right)) {
        (None, None) => {
            let v1 = parse_strict_i64(left)?;
            let v2 = parse_strict_i64(right)?;
            Some(Bytes::from(v1.wrapping_add(v2).to_string()))
        }
        (Some(MergeOp::MaxI64(a)), Some(MergeOp::MaxI64(b))) => {
            Some(encode(&MergeOp::MaxI64(a.max(b))))
        }
        (Some(MergeOp::MinI64(a)), Some(MergeOp::MinI64(b))) => {
            Some(encode(&MergeOp::MinI64(a.min(b))))
        }
        (Some(MergeOp::BitOr(a)), Some(MergeOp::BitOr(b))) => Some(encode(&MergeOp::BitOr(a | b))),
        (Some(MergeOp::BitAnd(a)), Some(MergeOp::BitAnd(b))) => {
            Some(encode(&MergeOp::BitAnd(a & b)))
        }
        (
            Some(MergeOp::AppendBytes {
                data: d1,
                max_len: m1,
            }),
            Some(MergeOp::AppendBytes {
                data: d2,
                max_len: m2,
            }),
        ) => combine_append(&d1, m1, &d2, m2),
        _ => None,
    }
}

fn combine_append(
    left: &[u8],
    left_max: Option<usize>,
    right: &[u8],
    right_max: Option<usize>,
) -> Option<Bytes> {
    let can_fold = match (left_max, right_max) {
        (None, _) => true,
        (Some(l1), Some(l2)) => l1 >= l2,
        (Some(_), None) => false,
    };
    if !can_fold {
        return None;
    }

    let mut joined = BytesMut::with_capacity(left.len().saturating_add(right.len()));
    joined.put_slice(left);
    joined.put_slice(right);
    let start = right_max.map_or(0, |limit| joined.len().saturating_sub(limit));
    let data = joined.split_off(start).freeze();
    Some(encode(&MergeOp::AppendBytes {
        data,
        max_len: right_max,
    }))
}

fn signed(tag: u8, kind: u8, value: i64) -> Vec<u8> {
    let mut bytes = vec![tag, kind];
    bytes.extend_from_slice(&value.to_le_bytes());
    bytes
}

fn unsigned(tag: u8, kind: u8, value: u64) -> Vec<u8> {
    let mut bytes = vec![tag, kind];
    bytes.extend_from_slice(&value.to_le_bytes());
    bytes
}

fn read_i64(source: &[u8]) -> Option<i64> {
    let bytes = <[u8; 8]>::try_from(source.get(..8)?).ok()?;
    Some(i64::from_le_bytes(bytes))
}

fn read_u64(source: &[u8]) -> Option<u64> {
    let bytes = <[u8; 8]>::try_from(source.get(..8)?).ok()?;
    Some(u64::from_le_bytes(bytes))
}

fn parse_strict_i64(bytes: &[u8]) -> Option<i64> {
    if let Ok(arr) = <[u8; 8]>::try_from(bytes) {
        return Some(i64::from_le_bytes(arr));
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

/// Parse a stored integer. `None` means zero, as in earlier releases.
fn parse_i64(value: Option<&[u8]>) -> i64 {
    let Some(value) = value else {
        return 0;
    };
    parse_strict_i64(value).unwrap_or(0)
}

fn parse_u64(value: Option<&[u8]>) -> u64 {
    let Some(value) = value else {
        return 0;
    };
    if let Ok(bytes) = <[u8; 8]>::try_from(value) {
        return u64::from_le_bytes(bytes);
    }
    std::str::from_utf8(value)
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(0)
}
