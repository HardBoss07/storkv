//! JSON operation translation for the WebSocket transport.

use bytes::Bytes;
use serde_json::Value;
use storkv_proto::{
    Request, WireDurability, WireMergeOp, WireRetention, WireScopeOptions, WireTxMutation,
};

use crate::transports::http::HttpState;
use crate::transports::http::json::decode_base64;

use super::UNKNOWN_OPERATION;

type WriteSet = Vec<((u32, Bytes), WireTxMutation)>;

pub(super) fn parse_operation(state: &HttpState, value: &Value) -> Result<Request, String> {
    let operation = value
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| UNKNOWN_OPERATION.to_string())?;
    let request = match operation {
        "ping" => Request::Ping,
        "health" => Request::HealthCheck,
        "telemetry" => Request::TelemetryQuery,
        "admin_stats" => Request::AdminStats,
        "tx_begin" => Request::TxBegin,
        "compact" => Request::Compact,
        "list_backups" => Request::ListBackups,
        "create_scope" => create_scope(value),
        "drop_scope" => Request::DropScope {
            name: required_str(value, "name")?.to_string(),
        },
        "checkpoint" => Request::Checkpoint {
            dest_dir: required_str(value, "dest_dir")?.to_string(),
        },
        "backup" => Request::TriggerBackup {
            wait: value.get("wait").and_then(Value::as_bool).unwrap_or(true),
        },
        "verify_backup" => Request::VerifyBackup {
            target_seq: value.get("target_seq").and_then(Value::as_u64),
        },
        "tx_commit" => tx_commit(state, value)?,
        "watch" => Request::WatchSubscribe,
        "watch_from" => Request::WatchFrom {
            from_seq: value.get("from_seq").and_then(Value::as_u64).unwrap_or(0),
        },
        "get" => Request::Get {
            scope_id: scope_id(state, value)?,
            key: key_field(value, "key", "key_b64")?,
        },
        "put" => Request::Put {
            scope_id: scope_id(state, value)?,
            key: key_field(value, "key", "key_b64")?,
            val: key_field(value, "value", "value_b64")?,
        },
        "delete" => Request::Delete {
            scope_id: scope_id(state, value)?,
            key: key_field(value, "key", "key_b64")?,
        },
        "delete_range" => Request::DeleteRange {
            scope_id: scope_id(state, value)?,
            start_key: required_key_field(value, "start", "start_b64")?,
            end_key: required_key_field(value, "end", "end_b64")?,
        },
        "merge" => merge(state, value)?,
        "scan" | "scan_prefix" => scan(state, value, operation)?,
        _ => return Err(UNKNOWN_OPERATION.to_string()),
    };
    Ok(request)
}

fn create_scope(value: &Value) -> Request {
    let prefix_bloom_len = value
        .get("prefix_bloom_len")
        .and_then(Value::as_u64)
        .and_then(|length| usize::try_from(length).ok());
    Request::CreateScope {
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        options: WireScopeOptions {
            durability: WireDurability::StrictFsync,
            retention: WireRetention::Permanent,
            prefix_bloom_len,
        },
    }
}

fn tx_commit(state: &HttpState, value: &Value) -> Result<Request, String> {
    let read_seq = value
        .get("read_seq")
        .and_then(Value::as_u64)
        .ok_or_else(|| UNKNOWN_OPERATION.to_string())?;
    let read_set = parse_read_set(state, value.get("read_set"))?;
    let write_set = parse_write_set(state, value.get("write_set"))?;
    Ok(Request::TxCommit {
        read_seq,
        read_set,
        write_set,
    })
}

fn parse_read_set(state: &HttpState, value: Option<&Value>) -> Result<Vec<(u32, Bytes)>, String> {
    let Some(entries) = value.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    entries
        .iter()
        .map(|entry| Ok((scope_id(state, entry)?, key_field(entry, "key", "key_b64")?)))
        .collect()
}

fn parse_write_set(state: &HttpState, value: Option<&Value>) -> Result<WriteSet, String> {
    let Some(entries) = value.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    entries
        .iter()
        .map(|entry| {
            let scope = scope_id(state, entry)?;
            let key = key_field(entry, "key", "key_b64")?;
            Ok(((scope, key), mutation(entry)?))
        })
        .collect()
}

fn mutation(entry: &Value) -> Result<WireTxMutation, String> {
    match entry.get("kind").and_then(Value::as_str).unwrap_or("put") {
        "delete" => Ok(WireTxMutation::Delete),
        "delete_range" => Ok(WireTxMutation::DeleteRange {
            end_key: required_key_field(entry, "end", "end_b64")?,
        }),
        "merge" => Ok(WireTxMutation::Merge {
            operand: key_field(entry, "operand", "operand_b64")?,
        }),
        _ => Ok(WireTxMutation::Put {
            val: key_field(entry, "value", "value_b64").unwrap_or_default(),
            expires_at_ms: entry
                .get("expires_at_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        }),
    }
}

fn merge(state: &HttpState, value: &Value) -> Result<Request, String> {
    let scope = scope_id(state, value)?;
    let key = key_field(value, "key", "key_b64")?;
    let merge_op = match value.get("op_kind").and_then(Value::as_str) {
        Some("custom") => WireMergeOp::Custom(key_field(value, "operand", "operand_b64")?),
        _ => WireMergeOp::CounterAdd(value.get("delta").and_then(Value::as_i64).unwrap_or(1)),
    };
    Ok(Request::Merge {
        scope_id: scope,
        key,
        op: merge_op,
    })
}

fn scan(state: &HttpState, value: &Value, operation: &str) -> Result<Request, String> {
    let scope_id = scope_id(state, value)?;
    if operation == "scan_prefix" {
        return Ok(Request::ScanPrefix {
            scope_id,
            prefix: key_field(value, "prefix", "prefix_b64")?,
        });
    }
    Ok(Request::Scan {
        scope_id,
        start_key: optional_key_field(value, "start", Some("start_b64")),
        end_key: optional_key_field(value, "end", Some("end_b64")),
    })
}

fn scope_id(state: &HttpState, value: &Value) -> Result<u32, String> {
    if let Some(scope_id) = value.get("scope_id").and_then(Value::as_u64) {
        return u32::try_from(scope_id).map_err(|_| UNKNOWN_OPERATION.to_string());
    }
    let name = value
        .get("scope")
        .and_then(Value::as_str)
        .ok_or_else(|| UNKNOWN_OPERATION.to_string())?;
    state
        .server
        .db()
        .inner()
        .scope_names
        .read()
        .get(name)
        .copied()
        .ok_or_else(|| format!("scope not found: {name}"))
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| UNKNOWN_OPERATION.to_string())
}

fn required_key_field(value: &Value, field: &str, field_b64: &str) -> Result<Bytes, String> {
    key_field(value, field, field_b64)
}

fn key_field(value: &Value, field: &str, field_b64: &str) -> Result<Bytes, String> {
    optional_key_field(value, field, Some(field_b64)).ok_or_else(|| UNKNOWN_OPERATION.to_string())
}

fn optional_key_field(value: &Value, field: &str, field_b64: Option<&str>) -> Option<Bytes> {
    if let Some(encoded) = field_b64
        .and_then(|name| value.get(name))
        .and_then(Value::as_str)
    {
        if let Some(decoded) = decode_base64(encoded) {
            return Some(Bytes::from(decoded));
        }
    }
    value
        .get(field)
        .and_then(Value::as_str)
        .map(|text| Bytes::copy_from_slice(text.as_bytes()))
}
