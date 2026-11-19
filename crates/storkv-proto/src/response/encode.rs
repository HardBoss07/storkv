use bytes::{BufMut, Bytes, BytesMut};

use super::types::{
    Response, WireAdminStats, WireBackupEntry, WireBackupKind, WireBackupReport, WireBackupStatus,
    WireBackupVerifyReport, WireChangeEvent, WireTelemetrySnapshot,
};
use crate::error::encode_error;
use crate::frame::{put_bytes, put_opt_bytes, put_str};

pub(crate) fn encode_response(destination: &mut BytesMut, response: &Response) {
    match response {
        Response::Pong => destination.put_u8(0),
        Response::Empty => destination.put_u8(1),
        Response::TxStarted { read_seq } => {
            destination.put_u8(8);
            destination.put_u64_le(*read_seq);
        }
        Response::Value(optional_value) => {
            destination.put_u8(2);
            put_opt_bytes(destination, optional_value.as_ref());
        }
        Response::Scan(pairs) => {
            destination.put_u8(3);
            encode_scan_pairs(destination, pairs);
        }
        Response::Scope(handle) => {
            destination.put_u8(4);
            destination.put_u32_le(handle.scope_id);
            put_str(destination, &handle.name);
        }
        Response::Telemetry(telemetry) => {
            destination.put_u8(5);
            encode_telemetry(destination, telemetry);
        }
        Response::HealthOk => destination.put_u8(6),
        Response::WatchEvent(event) => {
            destination.put_u8(7);
            encode_change_event(destination, event);
        }
        Response::AdminStats(stats) => {
            destination.put_u8(9);
            encode_admin_stats(destination, stats);
        }
        Response::BackupStatus(status) => {
            destination.put_u8(10);
            encode_backup_status(destination, status);
        }
        Response::BackupReport(report) => {
            destination.put_u8(11);
            encode_backup_report(destination, report);
        }
        Response::BackupVerifyReport(report) => {
            destination.put_u8(12);
            encode_backup_verify_report(destination, report);
        }
        Response::BackupCatalog(entries) => {
            destination.put_u8(13);
            encode_backup_list(destination, entries);
        }
        Response::KeyMeta(meta) => {
            destination.put_u8(14);
            match meta {
                Some(meta) => {
                    destination.put_u8(1);
                    destination.put_u64_le(meta.expires_at_ms);
                    destination.put_u64_le(meta.seq);
                }
                None => destination.put_u8(0),
            }
        }
        Response::Error(error) => {
            destination.put_u8(255);
            encode_error(destination, error);
        }
        Response::ReplicationFrame { sent_at_ms, frame } => {
            destination.put_u8(15);
            destination.put_u64_le(*sent_at_ms);
            put_bytes(destination, frame);
        }
        Response::ReplicationSnapshot(entries) => {
            destination.put_u8(16);
            destination.put_u32_le(entries.len() as u32);
            for (scope_id, key, value) in entries {
                destination.put_u32_le(*scope_id);
                put_bytes(destination, key);
                put_bytes(destination, value);
            }
        }
    }
}

fn encode_scan_pairs(destination: &mut BytesMut, pairs: &[(Bytes, Bytes)]) {
    destination.put_u32_le(pairs.len() as u32);
    for (user_key, entry_value) in pairs {
        put_bytes(destination, user_key);
        put_bytes(destination, entry_value);
    }
}

fn encode_telemetry(destination: &mut BytesMut, telemetry: &WireTelemetrySnapshot) {
    destination.put_u64_le(telemetry.writes_total);
    destination.put_u64_le(telemetry.reads_total);
    destination.put_u64_le(telemetry.write_conflicts_total);
    destination.put_u64_le(telemetry.compactions_total);
    destination.put_u64_le(telemetry.fsyncs_total);
    destination.put_u64_le(telemetry.backups_completed_total);
    destination.put_u64_le(telemetry.backups_failed_total);
    destination.put_u64_le(telemetry.last_backup_timestamp_ms);
    destination.put_u64_le(telemetry.last_backup_seq);
    destination.put_u64_le(telemetry.wal_segments_shipped_total);
}

fn encode_change_event(destination: &mut BytesMut, event: &WireChangeEvent) {
    match event {
        WireChangeEvent::Put {
            scope_id,
            key,
            value,
            seq,
        } => {
            destination.put_u8(1);
            destination.put_u32_le(*scope_id);
            put_bytes(destination, key);
            put_bytes(destination, value);
            destination.put_u64_le(*seq);
        }
        WireChangeEvent::Delete { scope_id, key, seq } => {
            destination.put_u8(2);
            destination.put_u32_le(*scope_id);
            put_bytes(destination, key);
            destination.put_u64_le(*seq);
        }
        WireChangeEvent::DropScope { scope_id, seq } => {
            destination.put_u8(3);
            destination.put_u32_le(*scope_id);
            destination.put_u64_le(*seq);
        }
        WireChangeEvent::InvalidateScope { scope_id, seq } => {
            destination.put_u8(4);
            destination.put_u32_le(*scope_id);
            destination.put_u64_le(*seq);
        }
    }
}

fn encode_admin_stats(destination: &mut BytesMut, stats: &WireAdminStats) {
    destination.put_u32_le(stats.levels.len() as u32);
    for level in &stats.levels {
        destination.put_u8(level.level);
        destination.put_u32_le(level.file_count);
        destination.put_u64_le(level.size_bytes);
    }
    destination.put_u32_le(stats.scopes.len() as u32);
    for scope in &stats.scopes {
        destination.put_u32_le(scope.scope_id);
        put_str(destination, &scope.name);
        destination.put_u64_le(scope.writes_total);
        destination.put_u64_le(scope.reads_total);
        destination.put_u32_le(scope.sst_files);
    }
    destination.put_u64_le(stats.fsync_latency_p99_us);
    destination.put_u64_le(stats.fsync_latency_samples);

    // Append-only extension: transport counters plus backup telemetry.
    destination.put_u8(1);
    destination.put_u32_le(stats.transports.len() as u32);
    for transport in &stats.transports {
        destination.put_u8(transport.kind);
        destination.put_u64_le(transport.connections_total);
        destination.put_u64_le(transport.active_connections);
        destination.put_u64_le(transport.requests_total);
    }
    destination.put_u64_le(stats.backups_completed_total);
    destination.put_u64_le(stats.backups_failed_total);
    destination.put_u64_le(stats.last_backup_timestamp_ms);
    destination.put_u64_le(stats.last_backup_seq);
    destination.put_u64_le(stats.wal_segments_shipped_total);
}

fn put_opt_str(destination: &mut BytesMut, value: Option<&str>) {
    match value {
        Some(text) => {
            destination.put_u8(1);
            put_str(destination, text);
        }
        None => destination.put_u8(0),
    }
}

fn encode_backup_report(destination: &mut BytesMut, report: &WireBackupReport) {
    put_str(destination, &report.generation_id);
    destination.put_u64_le(report.sequence);
    destination.put_u32_le(report.files);
    destination.put_u64_le(report.bytes);
    destination.put_u32_le(report.generations_removed);
    destination.put_u32_le(report.wal_segments_removed);
    destination.put_u64_le(report.bytes_freed);
}

fn encode_backup_status(destination: &mut BytesMut, status: &WireBackupStatus) {
    destination.put_u8(u8::from(status.enabled));
    put_opt_str(destination, status.backup_root.as_deref());
    destination.put_u64_le(status.last_backup_timestamp_ms);
    destination.put_u64_le(status.last_backup_seq);
    destination.put_u64_le(status.backups_completed_total);
    destination.put_u64_le(status.backups_failed_total);
    destination.put_u64_le(status.wal_segments_shipped_total);
    destination.put_u64_le(status.next_full_backup_in_ms);
    put_opt_str(destination, status.last_error.as_deref());
    put_str(destination, &status.sink);
}

fn encode_backup_verify_report(destination: &mut BytesMut, report: &WireBackupVerifyReport) {
    destination.put_u32_le(report.generations_checked);
    destination.put_u64_le(report.files_checked);
    destination.put_u64_le(report.bytes_checked);
    destination.put_u64_le(report.wal_segments_checked);
    destination.put_u32_le(report.errors.len() as u32);
    for error in &report.errors {
        put_str(destination, &error.path);
        put_str(destination, &error.detail);
    }
}

fn encode_backup_list(destination: &mut BytesMut, entries: &[WireBackupEntry]) {
    destination.put_u32_le(entries.len() as u32);
    for entry in entries {
        put_str(destination, &entry.generation_id);
        destination.put_u8(match entry.kind {
            WireBackupKind::Full => 1,
            WireBackupKind::Wal => 2,
        });
        destination.put_u64_le(entry.created_at_ms);
        destination.put_u64_le(entry.min_seq);
        destination.put_u64_le(entry.max_seq);
        destination.put_u64_le(entry.total_bytes);
        destination.put_u32_le(entry.files);
        put_str(destination, &entry.compression);
    }
}
