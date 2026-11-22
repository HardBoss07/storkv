use bytes::{Buf, BytesMut};
use std::time::Duration;

use super::decode_scope::decode_create_scope_request;
use super::decode_tx::decode_tx_commit_request;
use super::types::{Request, WireMergeOp, WirePutCondition};
use crate::error::StorkvWireError;
use crate::frame::{get_bytes, get_i64_le, get_opt_bytes, get_str, get_u8, get_u32_le, get_u64_le};

pub(crate) fn decode_request(source: &mut BytesMut) -> Result<Request, StorkvWireError> {
    if source.is_empty() {
        return Err(StorkvWireError::Protocol("Empty request payload".into()));
    }
    let opcode = source.get_u8();
    match opcode {
        0 => Ok(Request::Ping),
        1 => decode_keyed_request(source, RequestTag::Get),
        2 => decode_put_request(source),
        3 => decode_put_with_ttl_request(source),
        4 => decode_keyed_request(source, RequestTag::Delete),
        5 => decode_delete_range_request(source),
        6 => decode_merge_request(source),
        7 => decode_scan_request(source),
        8 => decode_scan_prefix_request(source),
        9 => decode_tx_commit_request(source),
        10 => decode_create_scope_request(source),
        11 => Ok(Request::DropScope {
            name: get_str(source)?,
        }),
        12 => Ok(Request::CreateSnapshot {
            name: get_str(source)?,
        }),
        13 => Ok(Request::UnpinSnapshot {
            name: get_str(source)?,
        }),
        14 => Ok(Request::Checkpoint {
            dest_dir: get_str(source)?,
        }),
        15 => Ok(Request::Compact),
        16 => Ok(Request::WatchSubscribe),
        17 => Ok(Request::TelemetryQuery),
        18 => Ok(Request::HealthCheck),
        19 => Ok(Request::TxBegin),
        20 => Ok(Request::AdminStats),
        21 => Ok(Request::TriggerBackup {
            wait: get_u8(source)? != 0,
        }),
        22 => Ok(Request::BackupStatus),
        23 => Ok(Request::VerifyBackup {
            target_seq: get_opt_u64(source)?,
        }),
        24 => Ok(Request::ListBackups),
        25 => Ok(Request::KeyMeta {
            scope_id: get_u32_le(source)?,
            key: get_bytes(source)?,
        }),
        26 => Ok(Request::WatchFrom {
            from_seq: get_u64_le(source)?,
        }),
        27 => Ok(Request::ReplicationStart {
            from_seq: get_u64_le(source)?,
        }),
        28 => {
            let mode = get_u8(source)?;
            let max_lag_ms = get_u64_le(source)?;
            Ok(Request::ConfigureConsistency {
                consistency: crate::Consistency::from_wire(mode, max_lag_ms)?,
            })
        }
        29 => decode_put_if_request(source),
        other => Err(StorkvWireError::Protocol(format!(
            "Unknown request opcode: {}",
            other
        ))),
    }
}

fn get_opt_u64(source: &mut BytesMut) -> Result<Option<u64>, StorkvWireError> {
    match get_u8(source)? {
        0 => Ok(None),
        1 => Ok(Some(get_u64_le(source)?)),
        _ => Err(StorkvWireError::Protocol("Invalid optional u64 tag".into())),
    }
}

enum RequestTag {
    Get,
    Delete,
}

fn decode_keyed_request(
    source: &mut BytesMut,
    tag: RequestTag,
) -> Result<Request, StorkvWireError> {
    let scope_id = get_u32_le(source)?;
    let user_key = get_bytes(source)?;
    Ok(match tag {
        RequestTag::Get => Request::Get {
            scope_id,
            key: user_key,
        },
        RequestTag::Delete => Request::Delete {
            scope_id,
            key: user_key,
        },
    })
}

fn decode_put_request(source: &mut BytesMut) -> Result<Request, StorkvWireError> {
    let scope_id = get_u32_le(source)?;
    let user_key = get_bytes(source)?;
    let entry_value = get_bytes(source)?;
    Ok(Request::Put {
        scope_id,
        key: user_key,
        val: entry_value,
    })
}

fn decode_put_with_ttl_request(source: &mut BytesMut) -> Result<Request, StorkvWireError> {
    let scope_id = get_u32_le(source)?;
    let user_key = get_bytes(source)?;
    let entry_value = get_bytes(source)?;
    let ttl_ms = get_u64_le(source)?;
    Ok(Request::PutWithTtl {
        scope_id,
        key: user_key,
        val: entry_value,
        ttl: Duration::from_millis(ttl_ms),
    })
}

fn decode_put_if_request(source: &mut BytesMut) -> Result<Request, StorkvWireError> {
    let scope_id = get_u32_le(source)?;
    let user_key = get_bytes(source)?;
    let condition = match get_u8(source)? {
        0 => WirePutCondition::Absent,
        1 => WirePutCondition::Present,
        2 => WirePutCondition::ExpectedSeq(get_u64_le(source)?),
        3 => WirePutCondition::ExpectedValue(get_bytes(source)?),
        _ => {
            return Err(StorkvWireError::Protocol(
                "Invalid put condition type".into(),
            ));
        }
    };
    let entry_value = get_bytes(source)?;
    let ttl = get_opt_u64(source)?.map(Duration::from_millis);
    Ok(Request::PutIf {
        scope_id,
        key: user_key,
        condition,
        val: entry_value,
        ttl,
    })
}

fn decode_delete_range_request(source: &mut BytesMut) -> Result<Request, StorkvWireError> {
    let scope_id = get_u32_le(source)?;
    let start_key = get_bytes(source)?;
    let end_key = get_bytes(source)?;
    Ok(Request::DeleteRange {
        scope_id,
        start_key,
        end_key,
    })
}

fn decode_merge_request(source: &mut BytesMut) -> Result<Request, StorkvWireError> {
    let scope_id = get_u32_le(source)?;
    let user_key = get_bytes(source)?;
    let op_type = get_u8(source)?;
    let merge_op = match op_type {
        1 => WireMergeOp::CounterAdd(get_i64_le(source)?),
        2 => WireMergeOp::Custom(get_bytes(source)?),
        _ => return Err(StorkvWireError::Protocol("Invalid merge op type".into())),
    };
    Ok(Request::Merge {
        scope_id,
        key: user_key,
        op: merge_op,
    })
}

fn decode_scan_request(source: &mut BytesMut) -> Result<Request, StorkvWireError> {
    let scope_id = get_u32_le(source)?;
    let start_key = get_opt_bytes(source)?;
    let end_key = get_opt_bytes(source)?;
    Ok(Request::Scan {
        scope_id,
        start_key,
        end_key,
    })
}

fn decode_scan_prefix_request(source: &mut BytesMut) -> Result<Request, StorkvWireError> {
    let scope_id = get_u32_le(source)?;
    let prefix = get_bytes(source)?;
    Ok(Request::ScanPrefix { scope_id, prefix })
}
