use bytes::{BufMut, Bytes, BytesMut};

use super::types::WireTxMutation;
use crate::frame::put_bytes;

pub(super) fn encode_read_set(destination: &mut BytesMut, read_set: &[(u32, Bytes)]) {
    destination.put_u32_le(read_set.len() as u32);
    for (scope_id, user_key) in read_set {
        destination.put_u32_le(*scope_id);
        put_bytes(destination, user_key);
    }
}

pub(super) fn encode_write_set(
    destination: &mut BytesMut,
    write_set: &[((u32, Bytes), WireTxMutation)],
) {
    destination.put_u32_le(write_set.len() as u32);
    for ((scope_id, user_key), mutation) in write_set {
        destination.put_u32_le(*scope_id);
        put_bytes(destination, user_key);
        encode_tx_mutation(destination, mutation);
    }
}

fn encode_tx_mutation(destination: &mut BytesMut, mutation: &WireTxMutation) {
    match mutation {
        WireTxMutation::Put { val, expires_at_ms } => {
            destination.put_u8(1);
            put_bytes(destination, val);
            destination.put_u64_le(*expires_at_ms);
        }
        WireTxMutation::Delete => destination.put_u8(2),
        WireTxMutation::Merge { operand } => {
            destination.put_u8(3);
            put_bytes(destination, operand);
        }
        WireTxMutation::DeleteRange { end_key } => {
            destination.put_u8(4);
            put_bytes(destination, end_key);
        }
    }
}
