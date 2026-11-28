//! Administrative and backup response decoding.
//!
//! Collection counts are checked against the remaining payload before the
//! decoder reserves memory. Trailing fields are append-only extensions.

use bytes::BytesMut;

use super::types::{
    WireAdminStats, WireBackupEntry, WireBackupKind, WireBackupReport, WireBackupStatus,
    WireBackupVerifyError, WireBackupVerifyReport, WireLevelStat, WireScopeStat, WireTransportStat,
};
use crate::error::StorkvWireError;
use crate::frame::{get_str, get_u8, get_u32_le, get_u64_le};

const U8_SIZE: usize = std::mem::size_of::<u8>();
const U32_SIZE: usize = std::mem::size_of::<u32>();
const U64_SIZE: usize = std::mem::size_of::<u64>();
const BACKUP_COUNTERS_SIZE: usize = 5 * U64_SIZE;
const TRANSPORT_STAT_MIN_SIZE: usize = U8_SIZE + 3 * U64_SIZE;
const LEVEL_STAT_MIN_SIZE: usize = U8_SIZE + U32_SIZE + U64_SIZE;
const SCOPE_STAT_MIN_SIZE: usize = 2 * U32_SIZE + 2 * U64_SIZE + U32_SIZE;

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
        if source.len() >= BACKUP_COUNTERS_SIZE {
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
    if count > source.len() / TRANSPORT_STAT_MIN_SIZE {
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
    if level_count > source.len() / LEVEL_STAT_MIN_SIZE {
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
    if scope_count > source.len() / SCOPE_STAT_MIN_SIZE {
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

fn get_opt_str(source: &mut BytesMut) -> Result<Option<String>, StorkvWireError> {
    match get_u8(source)? {
        0 => Ok(None),
        1 => Ok(Some(get_str(source)?)),
        _ => Err(StorkvWireError::Protocol(
            "Invalid optional string tag".into(),
        )),
    }
}

pub(super) fn decode_backup_report(
    source: &mut BytesMut,
) -> Result<WireBackupReport, StorkvWireError> {
    Ok(WireBackupReport {
        generation_id: get_str(source)?,
        sequence: get_u64_le(source)?,
        files: get_u32_le(source)?,
        bytes: get_u64_le(source)?,
        generations_removed: get_u32_le(source)?,
        wal_segments_removed: get_u32_le(source)?,
        bytes_freed: get_u64_le(source)?,
    })
}

pub(super) fn decode_backup_status(
    source: &mut BytesMut,
) -> Result<WireBackupStatus, StorkvWireError> {
    Ok(WireBackupStatus {
        enabled: get_u8(source)? != 0,
        backup_root: get_opt_str(source)?,
        last_backup_timestamp_ms: get_u64_le(source)?,
        last_backup_seq: get_u64_le(source)?,
        backups_completed_total: get_u64_le(source)?,
        backups_failed_total: get_u64_le(source)?,
        wal_segments_shipped_total: get_u64_le(source)?,
        next_full_backup_in_ms: get_u64_le(source)?,
        last_error: get_opt_str(source)?,
        sink: get_str(source)?,
    })
}

pub(super) fn decode_backup_verify_report(
    source: &mut BytesMut,
) -> Result<WireBackupVerifyReport, StorkvWireError> {
    let generations_checked = get_u32_le(source)?;
    let files_checked = get_u64_le(source)?;
    let bytes_checked = get_u64_le(source)?;
    let wal_segments_checked = get_u64_le(source)?;
    let error_count = get_u32_le(source)? as usize;
    if error_count > source.len() / 8 {
        return Err(StorkvWireError::Protocol(
            "Backup verification error count exceeds available payload".into(),
        ));
    }
    let mut errors = Vec::with_capacity(error_count);
    for _ in 0..error_count {
        errors.push(WireBackupVerifyError {
            path: get_str(source)?,
            detail: get_str(source)?,
        });
    }
    Ok(WireBackupVerifyReport {
        generations_checked,
        files_checked,
        bytes_checked,
        wal_segments_checked,
        errors,
    })
}

pub(super) fn decode_backup_list(
    source: &mut BytesMut,
) -> Result<Vec<WireBackupEntry>, StorkvWireError> {
    let entry_count = get_u32_le(source)? as usize;
    // Each entry has at least two u32 string prefixes plus fixed fields.
    if entry_count > source.len() / 24 {
        return Err(StorkvWireError::Protocol(
            "Backup entry count exceeds available payload".into(),
        ));
    }
    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        let generation_id = get_str(source)?;
        let kind = match get_u8(source)? {
            1 => WireBackupKind::Full,
            2 => WireBackupKind::Wal,
            _ => {
                return Err(StorkvWireError::Protocol("Invalid backup kind".into()));
            }
        };
        entries.push(WireBackupEntry {
            generation_id,
            kind,
            created_at_ms: get_u64_le(source)?,
            min_seq: get_u64_le(source)?,
            max_seq: get_u64_le(source)?,
            total_bytes: get_u64_le(source)?,
            files: get_u32_le(source)?,
            compression: get_str(source)?,
        });
    }
    Ok(entries)
}
