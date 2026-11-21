//! Conversions between generated protobuf messages and wire requests.

use std::time::Duration;

use storkv_proto::{
    WireCompactionStyle, WireDurability, WireRetention, WireScopeOptions, WireTxMutation,
};

use super::{
    ChangeEvent, MUTATION_DELETE, MUTATION_DELETE_RANGE, MUTATION_MERGE, Mutation,
    ProtoScopeOptions, TelemetryReply,
};
use crate::server::Server;

pub(super) fn telemetry_reply(server: &Server) -> TelemetryReply {
    let telemetry = server.db().telemetry();
    let relaxed = std::sync::atomic::Ordering::Relaxed;
    let status = server.db().backup_status();
    TelemetryReply {
        writes_total: telemetry.writes_total.load(relaxed),
        reads_total: telemetry.reads_total.load(relaxed),
        write_conflicts_total: telemetry.write_conflicts_total.load(relaxed),
        compactions_total: telemetry.compactions_total.load(relaxed),
        fsyncs_total: telemetry.fsyncs_total.load(relaxed),
        backups_completed_total: status
            .as_ref()
            .map(|status| status.backups_completed_total)
            .unwrap_or_else(|| telemetry.backups_completed_total.load(relaxed)),
        backups_failed_total: status
            .as_ref()
            .map(|status| status.backups_failed_total)
            .unwrap_or_else(|| telemetry.backups_failed_total.load(relaxed)),
        last_backup_timestamp_ms: telemetry.last_backup_timestamp_ms.load(relaxed),
        last_backup_seq: telemetry.last_backup_seq.load(relaxed),
        wal_segments_shipped_total: telemetry.wal_segments_shipped_total.load(relaxed),
    }
}

pub(super) fn scope_options(options: Option<ProtoScopeOptions>) -> WireScopeOptions {
    let options = options.unwrap_or_default();
    let durability = match options.durability {
        1 => WireDurability::Buffered {
            sync_interval: Duration::from_millis(options.sync_interval_ms),
        },
        _ => WireDurability::StrictFsync,
    };
    let retention = match options.retention {
        1 => WireRetention::Ttl {
            default_ttl: Duration::from_millis(options.default_ttl_ms),
            compaction_style: match options.compaction_style {
                1 => WireCompactionStyle::Fifo,
                _ => WireCompactionStyle::Leveled,
            },
        },
        _ => WireRetention::Permanent,
    };
    WireScopeOptions {
        durability,
        retention,
        prefix_bloom_len: if options.has_prefix_bloom {
            Some(options.prefix_bloom_len as usize)
        } else {
            None
        },
    }
}

pub(super) fn mutation(entry: Mutation) -> Option<WireTxMutation> {
    match entry.kind {
        MUTATION_DELETE => Some(WireTxMutation::Delete),
        MUTATION_MERGE => Some(WireTxMutation::Merge {
            operand: bytes::Bytes::from(entry.value),
        }),
        MUTATION_DELETE_RANGE => Some(WireTxMutation::DeleteRange {
            end_key: bytes::Bytes::from(entry.end_key),
        }),
        _ => Some(WireTxMutation::Put {
            val: bytes::Bytes::from(entry.value),
            expires_at_ms: entry.expires_at_ms,
        }),
    }
}

pub(super) fn optional_bytes(bytes: &[u8]) -> Option<bytes::Bytes> {
    if bytes.is_empty() {
        None
    } else {
        Some(bytes::Bytes::copy_from_slice(bytes))
    }
}

pub(super) fn fallback_scope(write_set: &[Mutation]) -> u32 {
    write_set.first().map(|entry| entry.scope_id).unwrap_or(0)
}

pub(super) fn convert_event(event: storkv_core::watch::ChangeEvent) -> Option<ChangeEvent> {
    match event {
        storkv_core::watch::ChangeEvent::Put {
            scope_id,
            key,
            value,
            seq,
            expires_at_ms,
        } => Some(ChangeEvent {
            kind: 0,
            scope_id,
            key: key.to_vec(),
            value: value.to_vec(),
            seq,
            expires_at_ms,
        }),
        storkv_core::watch::ChangeEvent::Delete { scope_id, key, seq } => Some(ChangeEvent {
            kind: 1,
            scope_id,
            key: key.to_vec(),
            value: Vec::new(),
            seq,
            expires_at_ms: 0,
        }),
        storkv_core::watch::ChangeEvent::DropScope { scope_id, seq } => Some(ChangeEvent {
            kind: 2,
            scope_id,
            key: Vec::new(),
            value: Vec::new(),
            seq,
            expires_at_ms: 0,
        }),
        storkv_core::watch::ChangeEvent::InvalidateScope { scope_id, seq } => Some(ChangeEvent {
            kind: 3,
            scope_id,
            key: Vec::new(),
            value: Vec::new(),
            seq,
            expires_at_ms: 0,
        }),
        _ => None,
    }
}
