use bytes::{Buf, BytesMut};

use super::decode_admin::decode_admin_stats;
use super::types::{
    Response, WireBackupEntry, WireBackupKind, WireBackupReport, WireBackupStatus,
    WireBackupVerifyError, WireBackupVerifyReport, WireChangeEvent, WireKeyMeta, WireScopeHandle,
    WireTelemetrySnapshot,
};

use crate::error::{StorkvWireError, decode_error};
use crate::frame::{get_bytes, get_opt_bytes, get_str, get_u8, get_u32_le, get_u64_le};

pub(crate) fn decode_response(source: &mut BytesMut) -> Result<Response, StorkvWireError> {
    if source.is_empty() {
        return Err(StorkvWireError::Protocol("Empty response payload".into()));
    }
    let opcode = source.get_u8();
    match opcode {
        0 => Ok(Response::Pong),
        1 => Ok(Response::Empty),
        2 => Ok(Response::Value(get_opt_bytes(source)?)),
        3 => decode_scan_response(source),
        4 => {
            let scope_id = get_u32_le(source)?;
            let name = get_str(source)?;
            Ok(Response::Scope(WireScopeHandle { scope_id, name }))
        }
        5 => Ok(Response::Telemetry(decode_telemetry(source)?)),
        6 => Ok(Response::HealthOk),
        7 => Ok(Response::WatchEvent(decode_change_event(source)?)),
        8 => Ok(Response::TxStarted {
            read_seq: get_u64_le(source)?,
        }),
        9 => Ok(Response::AdminStats(decode_admin_stats(source)?)),
        10 => Ok(Response::BackupStatus(decode_backup_status(source)?)),
        11 => Ok(Response::BackupReport(decode_backup_report(source)?)),
        12 => Ok(Response::BackupVerifyReport(decode_backup_verify_report(
            source,
        )?)),
        13 => Ok(Response::BackupCatalog(decode_backup_list(source)?)),
        14 => decode_key_meta(source),
        15 => Ok(Response::ReplicationFrame {
            sent_at_ms: get_u64_le(source)?,
            frame: get_bytes(source)?,
        }),
        16 => {
            let count = get_u32_le(source)? as usize;
            if count > source.len() / 12 {
                return Err(StorkvWireError::Protocol(
                    "Replication snapshot count exceeds available payload".into(),
                ));
            }
            let mut entries = Vec::with_capacity(count);
            for _ in 0..count {
                let scope_id = get_u32_le(source)?;
                let key = get_bytes(source)?;
                let value = get_bytes(source)?;
                entries.push((scope_id, key, value));
            }
            Ok(Response::ReplicationSnapshot(entries))
        }
        255 => Ok(Response::Error(decode_error(source)?)),
        other => Err(StorkvWireError::Protocol(format!(
            "Unknown response opcode: {}",
            other
        ))),
    }
}

fn decode_scan_response(source: &mut BytesMut) -> Result<Response, StorkvWireError> {
    let pair_count = get_u32_le(source)? as usize;
    // Each pair costs at least two u32 length prefixes.
    if pair_count > source.len() / 8 {
        return Err(StorkvWireError::Protocol(
            "Scan result count exceeds available payload".into(),
        ));
    }
    let mut pairs = Vec::with_capacity(pair_count);
    for _ in 0..pair_count {
        let user_key = get_bytes(source)?;
        let entry_value = get_bytes(source)?;
        pairs.push((user_key, entry_value));
    }
    Ok(Response::Scan(pairs))
}

fn decode_telemetry(source: &mut BytesMut) -> Result<WireTelemetrySnapshot, StorkvWireError> {
    let mut snapshot = WireTelemetrySnapshot {
        writes_total: get_u64_le(source)?,
        reads_total: get_u64_le(source)?,
        write_conflicts_total: get_u64_le(source)?,
        compactions_total: get_u64_le(source)?,
        fsyncs_total: get_u64_le(source)?,
        ..WireTelemetrySnapshot::default()
    };
    // Backup counters are an append-only extension; older peers omit them.
    if source.len() >= 40 {
        snapshot.backups_completed_total = get_u64_le(source)?;
        snapshot.backups_failed_total = get_u64_le(source)?;
        snapshot.last_backup_timestamp_ms = get_u64_le(source)?;
        snapshot.last_backup_seq = get_u64_le(source)?;
        snapshot.wal_segments_shipped_total = get_u64_le(source)?;
    }
    Ok(snapshot)
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

fn decode_backup_report(source: &mut BytesMut) -> Result<WireBackupReport, StorkvWireError> {
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

fn decode_backup_status(source: &mut BytesMut) -> Result<WireBackupStatus, StorkvWireError> {
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

fn decode_backup_verify_report(
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

fn decode_backup_list(source: &mut BytesMut) -> Result<Vec<WireBackupEntry>, StorkvWireError> {
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

fn decode_key_meta(source: &mut BytesMut) -> Result<Response, StorkvWireError> {
    match get_u8(source)? {
        0 => Ok(Response::KeyMeta(None)),
        1 => Ok(Response::KeyMeta(Some(WireKeyMeta {
            expires_at_ms: get_u64_le(source)?,
            seq: get_u64_le(source)?,
        }))),
        _ => Err(StorkvWireError::Protocol(
            "Invalid key metadata presence tag".into(),
        )),
    }
}

fn decode_change_event(source: &mut BytesMut) -> Result<WireChangeEvent, StorkvWireError> {
    let event_type = get_u8(source)?;
    match event_type {
        1 => {
            let scope_id = get_u32_le(source)?;
            let key = get_bytes(source)?;
            let value = get_bytes(source)?;
            let seq = get_u64_le(source)?;
            Ok(WireChangeEvent::Put {
                scope_id,
                key,
                value,
                seq,
            })
        }
        2 => {
            let scope_id = get_u32_le(source)?;
            let key = get_bytes(source)?;
            let seq = get_u64_le(source)?;
            Ok(WireChangeEvent::Delete { scope_id, key, seq })
        }
        3 => {
            let scope_id = get_u32_le(source)?;
            let seq = get_u64_le(source)?;
            Ok(WireChangeEvent::DropScope { scope_id, seq })
        }
        4 => {
            let scope_id = get_u32_le(source)?;
            let seq = get_u64_le(source)?;
            Ok(WireChangeEvent::InvalidateScope { scope_id, seq })
        }
        _ => Err(StorkvWireError::Protocol(
            "Unknown watch event variant".into(),
        )),
    }
}
