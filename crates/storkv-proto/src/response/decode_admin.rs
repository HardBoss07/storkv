use bytes::BytesMut;

use super::types::{WireAdminStats, WireLevelStat, WireScopeStat};
use crate::error::StorKvWireError;
use crate::frame::{get_str, get_u8, get_u32_le, get_u64_le};

pub(super) fn decode_admin_stats(source: &mut BytesMut) -> Result<WireAdminStats, StorKvWireError> {
    let levels = decode_level_stats(source)?;
    let scopes = decode_scope_stats(source)?;
    Ok(WireAdminStats {
        levels,
        scopes,
        fsync_latency_p99_us: get_u64_le(source)?,
        fsync_latency_samples: get_u64_le(source)?,
    })
}

fn decode_level_stats(source: &mut BytesMut) -> Result<Vec<WireLevelStat>, StorKvWireError> {
    let level_count = get_u32_le(source)? as usize;
    // A level entry is at least u8 + u32 + u64.
    if level_count > source.len() / 13 {
        return Err(StorKvWireError::Protocol(
            "Level count exceeds available payload".into(),
        ));
    }
    let mut levels = Vec::with_capacity(level_count);
    for _ in 0..level_count {
        levels.push(WireLevelStat {
            level: get_u8(source)?,
            file_count: get_u32_le(source)?,
            size_bytes: get_u64_le(source)?,
        });
    }
    Ok(levels)
}

fn decode_scope_stats(source: &mut BytesMut) -> Result<Vec<WireScopeStat>, StorKvWireError> {
    let scope_count = get_u32_le(source)? as usize;
    // A scope entry is at least u32 + u32 name length + u64 + u64 + u32.
    if scope_count > source.len() / 28 {
        return Err(StorKvWireError::Protocol(
            "Scope count exceeds available payload".into(),
        ));
    }
    let mut scopes = Vec::with_capacity(scope_count);
    for _ in 0..scope_count {
        scopes.push(WireScopeStat {
            scope_id: get_u32_le(source)?,
            name: get_str(source)?,
            writes_total: get_u64_le(source)?,
            reads_total: get_u64_le(source)?,
            sst_files: get_u32_le(source)?,
        });
    }
    Ok(scopes)
}
