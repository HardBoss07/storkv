use bytes::{BufMut, Bytes, BytesMut};

use super::types::{Response, WireAdminStats, WireChangeEvent, WireTelemetrySnapshot};
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
        Response::Error(error) => {
            destination.put_u8(255);
            encode_error(destination, error);
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
}
