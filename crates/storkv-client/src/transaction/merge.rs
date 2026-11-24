//! Client-side merge operand encoding for transaction writes.
//!
//! The stored operand format is owned by `storkv-core`. Keep this encoding in
//! sync with `crates/storkv-core/src/mvcc/operand.rs`.

use bytes::Bytes;
use storkv_proto::WireMergeOp;

const TAG: u8 = 0xFF;
const OPERAND_KIND_INDEX: usize = 1;
const APPEND_HAS_BOUND_INDEX: usize = 2;
const APPEND_UNBOUNDED_DATA_INDEX: usize = 3;
const APPEND_BOUNDED_HEADER_LEN: usize = APPEND_UNBOUNDED_DATA_INDEX + std::mem::size_of::<u64>();

pub(super) fn encode_merge_operand(op: WireMergeOp) -> Bytes {
    match op {
        WireMergeOp::CounterAdd(delta) => Bytes::from(delta.to_string()),
        WireMergeOp::Custom(operand) => operand,
        WireMergeOp::AppendBytes { data, max_len } => {
            let mut bytes = vec![TAG, 1, u8::from(max_len.is_some())];
            if let Some(max_len) = max_len {
                bytes.extend_from_slice(&max_len.to_le_bytes());
            }
            bytes.extend_from_slice(&data);
            Bytes::from(bytes)
        }
        WireMergeOp::MaxI64(value) => signed(TAG, 2, value),
        WireMergeOp::MinI64(value) => signed(TAG, 3, value),
        WireMergeOp::BitOr(bits) => unsigned(TAG, 4, bits),
        WireMergeOp::BitAnd(bits) => unsigned(TAG, 5, bits),
        _ => Bytes::new(),
    }
}

pub(super) fn apply_merge_operand(current: Option<Bytes>, operand: &[u8]) -> Bytes {
    if operand.first() != Some(&TAG) || operand.len() < 2 {
        let total = parse_i64(current.as_deref()).wrapping_add(parse_i64(Some(operand)));
        return Bytes::from(total.to_string());
    }
    match operand[OPERAND_KIND_INDEX] {
        1 => append(current, operand),
        2 => {
            let next = match current {
                Some(current) => parse_i64(Some(&current)).max(read_i64(&operand[2..])),
                None => read_i64(&operand[2..]),
            };
            Bytes::from(next.to_string())
        }
        3 => {
            let next = match current {
                Some(current) => parse_i64(Some(&current)).min(read_i64(&operand[2..])),
                None => read_i64(&operand[2..]),
            };
            Bytes::from(next.to_string())
        }
        4 => {
            let bits = parse_u64(current.as_deref()) | read_u64(&operand[2..]);
            Bytes::from(bits.to_string())
        }
        5 => {
            let bits = parse_u64(current.as_deref()) & read_u64(&operand[2..]);
            Bytes::from(bits.to_string())
        }
        _ => Bytes::default(),
    }
}

fn append(current: Option<Bytes>, operand: &[u8]) -> Bytes {
    let has_bound = operand.get(APPEND_HAS_BOUND_INDEX).copied().unwrap_or(0);
    let (max_len, data) = if has_bound == 0 {
        (
            None,
            operand
                .get(APPEND_UNBOUNDED_DATA_INDEX..)
                .unwrap_or_default(),
        )
    } else {
        let bound = read_u64(
            operand
                .get(APPEND_UNBOUNDED_DATA_INDEX..)
                .unwrap_or_default(),
        );
        (
            Some(usize::try_from(bound).unwrap_or(usize::MAX)),
            operand.get(APPEND_BOUNDED_HEADER_LEN..).unwrap_or_default(),
        )
    };
    let mut joined = current.unwrap_or_default().to_vec();
    joined.extend_from_slice(data);
    let start = max_len.map_or(0, |max_len| joined.len().saturating_sub(max_len));
    Bytes::from(joined.split_off(start))
}

fn signed(tag: u8, kind: u8, value: i64) -> Bytes {
    let mut bytes = vec![tag, kind];
    bytes.extend_from_slice(&value.to_le_bytes());
    Bytes::from(bytes)
}

fn unsigned(tag: u8, kind: u8, value: u64) -> Bytes {
    let mut bytes = vec![tag, kind];
    bytes.extend_from_slice(&value.to_le_bytes());
    Bytes::from(bytes)
}

fn read_i64(source: &[u8]) -> i64 {
    source
        .get(..8)
        .and_then(|slice| <[u8; 8]>::try_from(slice).ok())
        .map(i64::from_le_bytes)
        .unwrap_or(0)
}

fn read_u64(source: &[u8]) -> u64 {
    source
        .get(..8)
        .and_then(|slice| <[u8; 8]>::try_from(slice).ok())
        .map(u64::from_le_bytes)
        .unwrap_or(0)
}

fn parse_i64(value: Option<&[u8]>) -> i64 {
    let Some(value) = value else {
        return 0;
    };
    if let Ok(bytes) = <[u8; 8]>::try_from(value) {
        return i64::from_le_bytes(bytes);
    }
    std::str::from_utf8(value)
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(0)
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
