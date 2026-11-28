//! Administrative and backup response encoding.
//!
//! The layout matches the decoder in [`super::decode_admin`].

use bytes::{BufMut, BytesMut};

use super::types::{
    WireAdminStats, WireBackupEntry, WireBackupKind, WireBackupReport, WireBackupStatus,
    WireBackupVerifyReport,
};
use crate::frame::put_str;

pub(super) fn encode_admin_stats(destination: &mut BytesMut, stats: &WireAdminStats) {
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

pub(super) fn encode_backup_report(destination: &mut BytesMut, report: &WireBackupReport) {
    put_str(destination, &report.generation_id);
    destination.put_u64_le(report.sequence);
    destination.put_u32_le(report.files);
    destination.put_u64_le(report.bytes);
    destination.put_u32_le(report.generations_removed);
    destination.put_u32_le(report.wal_segments_removed);
    destination.put_u64_le(report.bytes_freed);
}

pub(super) fn encode_backup_status(destination: &mut BytesMut, status: &WireBackupStatus) {
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

pub(super) fn encode_backup_verify_report(
    destination: &mut BytesMut,
    report: &WireBackupVerifyReport,
) {
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

pub(super) fn encode_backup_list(destination: &mut BytesMut, entries: &[WireBackupEntry]) {
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
