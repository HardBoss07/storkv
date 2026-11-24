use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::error::StorkvWireError;
use crate::request::{Request, decode_request, encode_request};
use crate::response::{Response, decode_response, encode_response};

/// Protocol magic bytes: "STORKPRT"
pub const PROTOCOL_MAGIC: &[u8; 8] = b"STORKPRT";
/// Current protocol version
pub const PROTOCOL_VERSION: u16 = 1;
/// Maximum frame payload size (64 MiB)
pub const MAX_FRAME_SIZE: usize = 64 * 1024 * 1024;

/// Size of an unsigned 32-bit field.
const U32_SIZE: usize = 4;
const LENGTH_PREFIX_SIZE: usize = U32_SIZE;
/// Size of an unsigned 16-bit field.
const U16_SIZE: usize = 2;
/// Size of an unsigned 64-bit field.
const U64_SIZE: usize = 8;
/// Size of an unsigned 8-bit field.
const U8_SIZE: usize = 1;
const FRAME_HEADER_SIZE: usize = U64_SIZE + U16_SIZE + U64_SIZE + U8_SIZE;
const REQUEST_PAYLOAD_TAG: u8 = 1;
const RESPONSE_PAYLOAD_TAG: u8 = 2;

/// One decoded protocol frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Correlation identifier chosen by the sender.
    pub request_id: u64,
    /// Request or response body.
    pub payload: MessagePayload,
}

/// Body of a protocol frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessagePayload {
    /// A client request.
    Request(Request),
    /// A server response.
    Response(Response),
}

impl Message {
    /// Build a request message with the given correlation identifier.
    pub fn new_request(request_id: u64, request: Request) -> Self {
        Self {
            request_id,
            payload: MessagePayload::Request(request),
        }
    }

    /// Build a response message with the given correlation identifier.
    pub fn new_response(request_id: u64, response: Response) -> Self {
        Self {
            request_id,
            payload: MessagePayload::Response(response),
        }
    }

    /// Encode into length-prefixed bytes:
    /// [length: u32 BE][magic: 8B][version: 2B BE][request_id: 8B LE][payload_type: 1B][payload bytes]
    pub fn encode(&self) -> Bytes {
        let mut frame_body = BytesMut::new();
        write_frame_header(&mut frame_body, self.request_id, payload_tag(&self.payload));
        match &self.payload {
            MessagePayload::Request(request) => encode_request(&mut frame_body, request),
            MessagePayload::Response(response) => encode_response(&mut frame_body, response),
        }

        let mut framed = BytesMut::with_capacity(LENGTH_PREFIX_SIZE + frame_body.len());
        framed.put_u32(frame_body.len() as u32);
        framed.put_slice(&frame_body);
        framed.freeze()
    }

    /// Attempts to decode one message from a buffer.
    /// Returns Ok(Some(msg)) if a complete frame was read and consumed,
    /// Returns Ok(None) if more data is needed,
    /// Returns Err if frame is malformed or invalid.
    pub fn decode(source: &mut BytesMut) -> Result<Option<Self>, StorkvWireError> {
        let Some(frame_length) = read_frame_length(source)? else {
            return Ok(None);
        };
        if source.len() < LENGTH_PREFIX_SIZE + frame_length {
            return Ok(None);
        }

        source.advance(LENGTH_PREFIX_SIZE);
        let mut frame_bytes = source.split_to(frame_length);
        decode_frame_body(&mut frame_bytes).map(Some)
    }
}

fn payload_tag(payload: &MessagePayload) -> u8 {
    match payload {
        MessagePayload::Request(_) => REQUEST_PAYLOAD_TAG,
        MessagePayload::Response(_) => RESPONSE_PAYLOAD_TAG,
    }
}

fn read_frame_length(source: &BytesMut) -> Result<Option<usize>, StorkvWireError> {
    if source.len() < LENGTH_PREFIX_SIZE {
        return Ok(None);
    }
    let frame_length = u32::from_be_bytes([source[0], source[1], source[2], source[3]]) as usize;
    if frame_length > MAX_FRAME_SIZE {
        return Err(StorkvWireError::Protocol(format!(
            "Frame size {} exceeds maximum allowed {}",
            frame_length, MAX_FRAME_SIZE
        )));
    }
    Ok(Some(frame_length))
}

fn write_frame_header(destination: &mut BytesMut, request_id: u64, payload_tag: u8) {
    destination.put_slice(PROTOCOL_MAGIC);
    destination.put_u16(PROTOCOL_VERSION);
    destination.put_u64_le(request_id);
    destination.put_u8(payload_tag);
}

fn decode_frame_body(frame_bytes: &mut BytesMut) -> Result<Message, StorkvWireError> {
    if frame_bytes.len() < FRAME_HEADER_SIZE {
        return Err(StorkvWireError::Protocol(
            "Frame header too short".to_string(),
        ));
    }

    let mut magic = [0u8; 8];
    frame_bytes.copy_to_slice(&mut magic);
    if &magic != PROTOCOL_MAGIC {
        return Err(StorkvWireError::Protocol(
            "Invalid protocol magic".to_string(),
        ));
    }

    let version = frame_bytes.get_u16();
    if version != PROTOCOL_VERSION {
        return Err(StorkvWireError::Protocol(format!(
            "Unsupported protocol version: {}",
            version
        )));
    }

    let request_id = frame_bytes.get_u64_le();
    let payload_type = frame_bytes.get_u8();
    let payload = decode_payload(frame_bytes, payload_type)?;
    Ok(Message {
        request_id,
        payload,
    })
}

fn decode_payload(
    frame_bytes: &mut BytesMut,
    payload_type: u8,
) -> Result<MessagePayload, StorkvWireError> {
    match payload_type {
        REQUEST_PAYLOAD_TAG => Ok(MessagePayload::Request(decode_request(frame_bytes)?)),
        RESPONSE_PAYLOAD_TAG => Ok(MessagePayload::Response(decode_response(frame_bytes)?)),
        other => Err(StorkvWireError::Protocol(format!(
            "Unknown payload type: {}",
            other
        ))),
    }
}

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
