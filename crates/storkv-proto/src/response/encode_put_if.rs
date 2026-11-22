use bytes::{BufMut, BytesMut};

use super::types::WirePutIfResult;
use crate::frame::put_opt_bytes;

pub(super) fn encode_put_if_result(destination: &mut BytesMut, result: &WirePutIfResult) {
    match result {
        WirePutIfResult::Applied { seq } => {
            destination.put_u8(0);
            destination.put_u64_le(*seq);
        }
        WirePutIfResult::Conflict {
            current_seq,
            current_value,
        } => {
            destination.put_u8(1);
            match current_seq {
                Some(seq) => {
                    destination.put_u8(1);
                    destination.put_u64_le(*seq);
                }
                None => destination.put_u8(0),
            }
            put_opt_bytes(destination, current_value.as_ref());
        }
    }
}
