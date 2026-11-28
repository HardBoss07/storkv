//! Scan request decoding.
//!
//! Optional bounds use a presence tag. Scan flags are an optional trailer, so
//! older peers can omit them.

use bytes::{Buf, BytesMut};

use super::types::Request;
use crate::error::StorkvWireError;
use crate::frame::{get_bytes, get_opt_bytes, get_u8, get_u32_le};

pub(super) fn decode_scan_flags(
    source: &mut BytesMut,
) -> Result<(Option<u32>, bool, bool), StorkvWireError> {
    if source.remaining() == 0 {
        return Ok((None, false, false));
    }
    let flags = get_u8(source)?;
    let limit = if flags & 1 != 0 {
        Some(get_u32_le(source)?)
    } else {
        None
    };
    Ok((limit, flags & 2 != 0, flags & 4 != 0))
}

pub(super) fn decode_scan_request(source: &mut BytesMut) -> Result<Request, StorkvWireError> {
    let scope_id = get_u32_le(source)?;
    let start_key = get_opt_bytes(source)?;
    let end_key = get_opt_bytes(source)?;
    let (limit, reverse, keys_only) = decode_scan_flags(source)?;
    Ok(Request::Scan {
        scope_id,
        start_key,
        end_key,
        limit,
        reverse,
        keys_only,
    })
}

pub(super) fn decode_scan_prefix_request(
    source: &mut BytesMut,
) -> Result<Request, StorkvWireError> {
    let scope_id = get_u32_le(source)?;
    let prefix = get_bytes(source)?;
    let (limit, reverse, keys_only) = decode_scan_flags(source)?;
    Ok(Request::ScanPrefix {
        scope_id,
        prefix,
        limit,
        reverse,
        keys_only,
    })
}
