//! Primitive Little Endian field codecs for frame payloads.
//!
//! Each getter checks the remaining length before it reads. Each setter writes
//! a Little Endian value.

use bytes::{Buf, BufMut, Bytes, BytesMut};

use super::{U32_SIZE, U64_SIZE};
use crate::error::StorkvWireError;

pub(crate) fn put_bytes<B: AsRef<[u8]>>(destination: &mut BytesMut, data: B) {
    let slice = data.as_ref();
    destination.put_u32_le(slice.len() as u32);
    destination.put_slice(slice);
}

pub(crate) fn put_opt_bytes<B: AsRef<[u8]>>(destination: &mut BytesMut, data: Option<B>) {
    match data {
        Some(slice) => {
            destination.put_u8(1);
            put_bytes(destination, slice);
        }
        None => destination.put_u8(0),
    }
}

pub(crate) fn put_str(destination: &mut BytesMut, text: &str) {
    put_bytes(destination, text.as_bytes());
}

pub(crate) fn get_u8(source: &mut BytesMut) -> Result<u8, StorkvWireError> {
    if source.is_empty() {
        return Err(StorkvWireError::Protocol(
            "Unexpected EOF reading u8".into(),
        ));
    }
    Ok(source.get_u8())
}

pub(crate) fn get_u32_le(source: &mut BytesMut) -> Result<u32, StorkvWireError> {
    if source.len() < U32_SIZE {
        return Err(StorkvWireError::Protocol(
            "Unexpected EOF reading u32".into(),
        ));
    }
    Ok(source.get_u32_le())
}

pub(crate) fn get_i64_le(source: &mut BytesMut) -> Result<i64, StorkvWireError> {
    if source.len() < U64_SIZE {
        return Err(StorkvWireError::Protocol(
            "Unexpected EOF reading i64".into(),
        ));
    }
    Ok(source.get_i64_le())
}

pub(crate) fn get_u64_le(source: &mut BytesMut) -> Result<u64, StorkvWireError> {
    if source.len() < U64_SIZE {
        return Err(StorkvWireError::Protocol(
            "Unexpected EOF reading u64".into(),
        ));
    }
    Ok(source.get_u64_le())
}

pub(crate) fn get_bytes(source: &mut BytesMut) -> Result<Bytes, StorkvWireError> {
    let payload_length = get_u32_le(source)? as usize;
    if source.len() < payload_length {
        return Err(StorkvWireError::Protocol(
            "Unexpected EOF reading bytes".into(),
        ));
    }
    Ok(source.split_to(payload_length).freeze())
}

pub(crate) fn get_opt_bytes(source: &mut BytesMut) -> Result<Option<Bytes>, StorkvWireError> {
    match get_u8(source)? {
        0 => Ok(None),
        1 => Ok(Some(get_bytes(source)?)),
        _ => Err(StorkvWireError::Protocol(
            "Invalid optional bytes tag".into(),
        )),
    }
}

pub(crate) fn get_str(source: &mut BytesMut) -> Result<String, StorkvWireError> {
    let text_bytes = get_bytes(source)?;
    String::from_utf8(text_bytes.to_vec())
        .map_err(|error| StorkvWireError::Protocol(format!("Invalid UTF-8: {}", error)))
}
