//! Merge operator serialization for requests.

use bytes::{BufMut, BytesMut};

use super::types::WireMergeOp;
use crate::frame::put_bytes;

pub(super) fn encode_merge_op(destination: &mut BytesMut, merge_op: &WireMergeOp) {
    match merge_op {
        WireMergeOp::CounterAdd(delta) => {
            destination.put_u8(1);
            destination.put_i64_le(*delta);
        }
        WireMergeOp::Custom(operand) => {
            destination.put_u8(2);
            put_bytes(destination, operand);
        }
        WireMergeOp::AppendBytes { data, max_len } => {
            destination.put_u8(3);
            match max_len {
                Some(max_len) => {
                    destination.put_u8(1);
                    destination.put_u64_le(*max_len);
                }
                None => destination.put_u8(0),
            }
            put_bytes(destination, data);
        }
        WireMergeOp::MaxI64(value) => {
            destination.put_u8(4);
            destination.put_i64_le(*value);
        }
        WireMergeOp::MinI64(value) => {
            destination.put_u8(5);
            destination.put_i64_le(*value);
        }
        WireMergeOp::BitOr(bits) => {
            destination.put_u8(6);
            destination.put_u64_le(*bits);
        }
        WireMergeOp::BitAnd(bits) => {
            destination.put_u8(7);
            destination.put_u64_le(*bits);
        }
    }
}
