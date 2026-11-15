use bytes::{Buf, BytesMut};

use super::decode_admin::decode_admin_stats;
use super::types::{Response, WireChangeEvent, WireScopeHandle, WireTelemetrySnapshot};
use crate::error::{StorKvWireError, decode_error};
use crate::frame::{get_bytes, get_opt_bytes, get_str, get_u8, get_u32_le, get_u64_le};

pub(crate) fn decode_response(source: &mut BytesMut) -> Result<Response, StorKvWireError> {
    if source.is_empty() {
        return Err(StorKvWireError::Protocol("Empty response payload".into()));
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
        255 => Ok(Response::Error(decode_error(source)?)),
        other => Err(StorKvWireError::Protocol(format!(
            "Unknown response opcode: {}",
            other
        ))),
    }
}

fn decode_scan_response(source: &mut BytesMut) -> Result<Response, StorKvWireError> {
    let pair_count = get_u32_le(source)? as usize;
    // Each pair costs at least two u32 length prefixes.
    if pair_count > source.len() / 8 {
        return Err(StorKvWireError::Protocol(
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

fn decode_telemetry(source: &mut BytesMut) -> Result<WireTelemetrySnapshot, StorKvWireError> {
    Ok(WireTelemetrySnapshot {
        writes_total: get_u64_le(source)?,
        reads_total: get_u64_le(source)?,
        write_conflicts_total: get_u64_le(source)?,
        compactions_total: get_u64_le(source)?,
        fsyncs_total: get_u64_le(source)?,
    })
}

fn decode_change_event(source: &mut BytesMut) -> Result<WireChangeEvent, StorKvWireError> {
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
        _ => Err(StorKvWireError::Protocol(
            "Unknown watch event variant".into(),
        )),
    }
}
