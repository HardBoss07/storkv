use bytes::BytesMut;

use super::types::{WireAdminStats, WireLevelStat, WireScopeStat, WireTransportStat};
use crate::error::StorkvWireError;
use crate::frame::{get_str, get_u8, get_u32_le, get_u64_le};

pub(super) fn decode_admin_stats(source: &mut BytesMut) -> Result<WireAdminStats, StorkvWireError> {
    let levels = decode_level_stats(source)?;
    let scopes = decode_scope_stats(source)?;
    let mut stats = WireAdminStats {
        levels,
        scopes,
        fsync_latency_p99_us: get_u64_le(source)?,
        fsync_latency_samples: get_u64_le(source)?,
        ..WireAdminStats::default()
    };
    // Append-only extension; older peers end the payload here.
    if !source.is_empty() && get_u8(source)? == 1 {
        stats.transports = decode_transport_stats(source)?;
        if source.len() >= 40 {
            stats.backups_completed_total = get_u64_le(source)?;
            stats.backups_failed_total = get_u64_le(source)?;
            stats.last_backup_timestamp_ms = get_u64_le(source)?;
            stats.last_backup_seq = get_u64_le(source)?;
            stats.wal_segments_shipped_total = get_u64_le(source)?;
        }
    }
    Ok(stats)
}

fn decode_transport_stats(
    source: &mut BytesMut,
) -> Result<Vec<WireTransportStat>, StorkvWireError> {
    let count = get_u32_le(source)? as usize;
    // Each entry is u8 + three u64 fields.
    if count > source.len() / 25 {
        return Err(StorkvWireError::Protocol(
            "Transport count exceeds available payload".into(),
        ));
    }
    let mut transports = Vec::with_capacity(count);
    for _ in 0..count {
        transports.push(WireTransportStat {
            kind: get_u8(source)?,
            connections_total: get_u64_le(source)?,
            active_connections: get_u64_le(source)?,
            requests_total: get_u64_le(source)?,
        });
    }
    Ok(transports)
}

fn decode_level_stats(source: &mut BytesMut) -> Result<Vec<WireLevelStat>, StorkvWireError> {
    let level_count = get_u32_le(source)? as usize;
    // A level entry is at least u8 + u32 + u64.
    if level_count > source.len() / 13 {
        return Err(StorkvWireError::Protocol(
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

fn decode_scope_stats(source: &mut BytesMut) -> Result<Vec<WireScopeStat>, StorkvWireError> {
    let scope_count = get_u32_le(source)? as usize;
    // A scope entry is at least u32 + u32 name length + u64 + u64 + u32.
    if scope_count > source.len() / 28 {
        return Err(StorkvWireError::Protocol(
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
