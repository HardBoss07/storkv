use bytes::BytesMut;
use std::time::Duration;

use super::types::{Request, WireCompactionStyle, WireDurability, WireRetention, WireScopeOptions};
use crate::error::StorKvWireError;
use crate::frame::{get_str, get_u8, get_u32_le, get_u64_le};

pub(crate) fn decode_create_scope_request(
    source: &mut BytesMut,
) -> Result<Request, StorKvWireError> {
    let name = get_str(source)?;
    let options = decode_scope_options(source)?;
    Ok(Request::CreateScope { name, options })
}

fn decode_scope_options(source: &mut BytesMut) -> Result<WireScopeOptions, StorKvWireError> {
    let durability = decode_durability(source)?;
    let retention = decode_retention(source)?;
    let prefix_bloom_len = decode_prefix_bloom_len(source)?;
    Ok(WireScopeOptions {
        durability,
        retention,
        prefix_bloom_len,
    })
}

fn decode_durability(source: &mut BytesMut) -> Result<WireDurability, StorKvWireError> {
    match get_u8(source)? {
        1 => Ok(WireDurability::StrictFsync),
        2 => {
            let interval_ms = get_u64_le(source)?;
            Ok(WireDurability::Buffered {
                sync_interval: Duration::from_millis(interval_ms),
            })
        }
        _ => Err(StorKvWireError::Protocol("Invalid durability".into())),
    }
}

fn decode_retention(source: &mut BytesMut) -> Result<WireRetention, StorKvWireError> {
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
        _ => Err(StorKvWireError::Protocol("Invalid retention".into())),
    }
}

fn decode_compaction_style(source: &mut BytesMut) -> Result<WireCompactionStyle, StorKvWireError> {
    match get_u8(source)? {
        1 => Ok(WireCompactionStyle::Leveled),
        2 => Ok(WireCompactionStyle::Fifo),
        _ => Err(StorKvWireError::Protocol("Invalid compaction".into())),
    }
}

fn decode_prefix_bloom_len(source: &mut BytesMut) -> Result<Option<usize>, StorKvWireError> {
    match get_u8(source)? {
        0 => Ok(None),
        1 => Ok(Some(get_u32_le(source)? as usize)),
        _ => Err(StorKvWireError::Protocol("Invalid bloom tag".into())),
    }
}
