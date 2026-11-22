use bytes::{BufMut, BytesMut};

use super::encode_tx;
use super::types::{
    Request, WireCompactionStyle, WireDurability, WireMergeOp, WireRetention, WireScopeOptions,
};
use crate::frame::{put_bytes, put_opt_bytes, put_str};

pub(crate) fn encode_request(destination: &mut BytesMut, request: &Request) {
    match request {
        Request::Ping => destination.put_u8(0),
        Request::Get { scope_id, key } => {
            destination.put_u8(1);
            destination.put_u32_le(*scope_id);
            put_bytes(destination, key);
        }
        Request::Put { scope_id, key, val } => {
            destination.put_u8(2);
            destination.put_u32_le(*scope_id);
            put_bytes(destination, key);
            put_bytes(destination, val);
        }
        Request::PutWithTtl {
            scope_id,
            key,
            val,
            ttl,
        } => {
            destination.put_u8(3);
            destination.put_u32_le(*scope_id);
            put_bytes(destination, key);
            put_bytes(destination, val);
            destination.put_u64_le(ttl.as_millis() as u64);
        }
        Request::PutIf {
            scope_id,
            key,
            condition,
            val,
            ttl,
        } => {
            destination.put_u8(29);
            destination.put_u32_le(*scope_id);
            put_bytes(destination, key);
            super::types::encode_put_condition(destination, condition);
            put_bytes(destination, val);
            put_opt_u64(destination, ttl.as_ref().map(|ttl| ttl.as_millis() as u64));
        }
        Request::Delete { scope_id, key } => {
            destination.put_u8(4);
            destination.put_u32_le(*scope_id);
            put_bytes(destination, key);
        }
        Request::DeleteRange {
            scope_id,
            start_key,
            end_key,
        } => {
            destination.put_u8(5);
            destination.put_u32_le(*scope_id);
            put_bytes(destination, start_key);
            put_bytes(destination, end_key);
        }
        Request::Merge { scope_id, key, op } => {
            destination.put_u8(6);
            destination.put_u32_le(*scope_id);
            put_bytes(destination, key);
            encode_merge_op(destination, op);
        }
        Request::Scan {
            scope_id,
            start_key,
            end_key,
        } => {
            destination.put_u8(7);
            destination.put_u32_le(*scope_id);
            put_opt_bytes(destination, start_key.as_ref());
            put_opt_bytes(destination, end_key.as_ref());
        }
        Request::ScanPrefix { scope_id, prefix } => {
            destination.put_u8(8);
            destination.put_u32_le(*scope_id);
            put_bytes(destination, prefix);
        }
        Request::TxCommit {
            read_seq,
            read_set,
            write_set,
        } => {
            destination.put_u8(9);
            destination.put_u64_le(*read_seq);
            encode_tx::encode_read_set(destination, read_set);
            encode_tx::encode_write_set(destination, write_set);
        }
        Request::CreateScope { name, options } => {
            destination.put_u8(10);
            put_str(destination, name);
            encode_scope_options(destination, options);
        }
        Request::DropScope { name } => {
            destination.put_u8(11);
            put_str(destination, name);
        }
        Request::CreateSnapshot { name } => {
            destination.put_u8(12);
            put_str(destination, name);
        }
        Request::UnpinSnapshot { name } => {
            destination.put_u8(13);
            put_str(destination, name);
        }
        Request::Checkpoint { dest_dir } => {
            destination.put_u8(14);
            put_str(destination, dest_dir);
        }
        Request::Compact => destination.put_u8(15),
        Request::WatchSubscribe => destination.put_u8(16),
        Request::TelemetryQuery => destination.put_u8(17),
        Request::HealthCheck => destination.put_u8(18),
        Request::TxBegin => destination.put_u8(19),
        Request::AdminStats => destination.put_u8(20),
        Request::TriggerBackup { wait } => {
            destination.put_u8(21);
            destination.put_u8(u8::from(*wait));
        }
        Request::BackupStatus => destination.put_u8(22),
        Request::VerifyBackup { target_seq } => {
            destination.put_u8(23);
            put_opt_u64(destination, *target_seq);
        }
        Request::ListBackups => destination.put_u8(24),
        Request::KeyMeta { scope_id, key } => {
            destination.put_u8(25);
            destination.put_u32_le(*scope_id);
            put_bytes(destination, key);
        }
        Request::WatchFrom { from_seq } => {
            destination.put_u8(26);
            destination.put_u64_le(*from_seq);
        }
        Request::ReplicationStart { from_seq } => {
            destination.put_u8(27);
            destination.put_u64_le(*from_seq);
        }
        Request::ConfigureConsistency { consistency } => {
            destination.put_u8(28);
            let (mode, max_lag_ms) = consistency.to_wire();
            destination.put_u8(mode);
            destination.put_u64_le(max_lag_ms);
        }
    }
}

fn put_opt_u64(destination: &mut BytesMut, value: Option<u64>) {
    match value {
        Some(seq) => {
            destination.put_u8(1);
            destination.put_u64_le(seq);
        }
        None => destination.put_u8(0),
    }
}

fn encode_merge_op(destination: &mut BytesMut, merge_op: &WireMergeOp) {
    match merge_op {
        WireMergeOp::CounterAdd(delta) => {
            destination.put_u8(1);
            destination.put_i64_le(*delta);
        }
        WireMergeOp::Custom(operand) => {
            destination.put_u8(2);
            put_bytes(destination, operand);
        }
    }
}

fn encode_scope_options(destination: &mut BytesMut, options: &WireScopeOptions) {
    match &options.durability {
        WireDurability::StrictFsync => destination.put_u8(1),
        WireDurability::Buffered { sync_interval } => {
            destination.put_u8(2);
            destination.put_u64_le(sync_interval.as_millis() as u64);
        }
    }
    match &options.retention {
        WireRetention::Permanent => destination.put_u8(1),
        WireRetention::Ttl {
            default_ttl,
            compaction_style,
        } => {
            destination.put_u8(2);
            destination.put_u64_le(default_ttl.as_millis() as u64);
            destination.put_u8(match compaction_style {
                WireCompactionStyle::Leveled => 1,
                WireCompactionStyle::Fifo => 2,
            });
        }
    }
    match options.prefix_bloom_len {
        Some(filter_len) => {
            destination.put_u8(1);
            destination.put_u32_le(filter_len as u32);
        }
        None => destination.put_u8(0),
    }
    let mut flags = u8::from(options.allow_permanent);
    if options.min_ttl.is_some() {
        flags |= 2;
    }
    if options.max_ttl.is_some() {
        flags |= 4;
    }
    destination.put_u8(flags);
    if let Some(min_ttl) = options.min_ttl {
        destination.put_u64_le(min_ttl.as_millis() as u64);
    }
    if let Some(max_ttl) = options.max_ttl {
        destination.put_u64_le(max_ttl.as_millis() as u64);
    }
}
