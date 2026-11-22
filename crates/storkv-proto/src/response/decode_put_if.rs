use bytes::BytesMut;

use super::types::WirePutIfResult;
use crate::error::StorkvWireError;
use crate::frame::{get_opt_bytes, get_u8, get_u64_le};

pub(super) fn decode_put_if_result(
    source: &mut BytesMut,
) -> Result<WirePutIfResult, StorkvWireError> {
    match get_u8(source)? {
        0 => Ok(WirePutIfResult::Applied {
            seq: get_u64_le(source)?,
        }),
        1 => Ok(WirePutIfResult::Conflict {
            current_seq: get_opt_u64(source)?,
            current_value: get_opt_bytes(source)?,
        }),
        _ => Err(StorkvWireError::Protocol(
            "Invalid put-if result type".into(),
        )),
    }
}

fn get_opt_u64(source: &mut BytesMut) -> Result<Option<u64>, StorkvWireError> {
    match get_u8(source)? {
        0 => Ok(None),
        1 => Ok(Some(get_u64_le(source)?)),
        _ => Err(StorkvWireError::Protocol("Invalid optional u64 tag".into())),
    }
}
