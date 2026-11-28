//! Scope creation request and scope option decoding.
//!
//! Older peers omit the TTL guardrail trailer. The decoder keeps the default
//! values when the trailer is absent.

use bytes::{Buf, BytesMut};
use std::time::Duration;

use super::types::{Request, WireCompactionStyle, WireDurability, WireRetention, WireScopeOptions};
use crate::error::StorkvWireError;
use crate::frame::{get_str, get_u8, get_u32_le, get_u64_le};

pub(crate) fn decode_create_scope_request(
    source: &mut BytesMut,
) -> Result<Request, StorkvWireError> {
    let name = get_str(source)?;
    let options = decode_scope_options(source)?;
    Ok(Request::CreateScope { name, options })
}

fn decode_scope_options(source: &mut BytesMut) -> Result<WireScopeOptions, StorkvWireError> {
    let durability = decode_durability(source)?;
    let retention = decode_retention(source)?;
    let prefix_bloom_len = decode_prefix_bloom_len(source)?;
    let (min_ttl, max_ttl, allow_permanent, active_ttl_tracking) = if source.remaining() > 0 {
        decode_ttl_guardrails(source)?
    } else {
        (None, None, true, false)
    };
    Ok(WireScopeOptions {
        durability,
        retention,
        prefix_bloom_len,
        min_ttl,
        max_ttl,
        allow_permanent,
        active_ttl_tracking,
    })
}

fn decode_ttl_guardrails(
    source: &mut BytesMut,
) -> Result<(Option<Duration>, Option<Duration>, bool, bool), StorkvWireError> {
    let flags = get_u8(source)?;
    let min_ttl = if flags & 2 != 0 {
        Some(Duration::from_millis(get_u64_le(source)?))
    } else {
        None
    };
    let max_ttl = if flags & 4 != 0 {
        Some(Duration::from_millis(get_u64_le(source)?))
    } else {
        None
    };
    Ok((min_ttl, max_ttl, flags & 1 != 0, flags & 8 != 0))
}

fn decode_durability(source: &mut BytesMut) -> Result<WireDurability, StorkvWireError> {
    match get_u8(source)? {
        1 => Ok(WireDurability::StrictFsync),
        2 => {
            let interval_ms = get_u64_le(source)?;
            Ok(WireDurability::Buffered {
                sync_interval: Duration::from_millis(interval_ms),
            })
        }
        _ => Err(StorkvWireError::Protocol("Invalid durability".into())),
    }
}

fn decode_retention(source: &mut BytesMut) -> Result<WireRetention, StorkvWireError> {
    match get_u8(source)? {
        1 => Ok(WireRetention::Permanent),
        2 => {
            let ttl_ms = get_u64_le(source)?;
            let compaction_style = decode_compaction_style(source)?;
            Ok(WireRetention::Ttl {
                default_ttl: Duration::from_millis(ttl_ms),
                compaction_style,
            })
        }
        _ => Err(StorkvWireError::Protocol("Invalid retention".into())),
    }
}

fn decode_compaction_style(source: &mut BytesMut) -> Result<WireCompactionStyle, StorkvWireError> {
    match get_u8(source)? {
        1 => Ok(WireCompactionStyle::Leveled),
        2 => Ok(WireCompactionStyle::Fifo),
        _ => Err(StorkvWireError::Protocol("Invalid compaction".into())),
    }
}

fn decode_prefix_bloom_len(source: &mut BytesMut) -> Result<Option<usize>, StorkvWireError> {
    match get_u8(source)? {
        0 => Ok(None),
        1 => Ok(Some(get_u32_le(source)? as usize)),
        _ => Err(StorkvWireError::Protocol("Invalid bloom tag".into())),
    }
}
