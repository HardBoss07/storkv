//! Cursor encoding for Redis `SCAN`.

use bytes::Bytes;

/// Number of hexadecimal characters that encode one byte.
const HEX_DIGITS_PER_BYTE: usize = 2;
/// Radix of the hexadecimal cursor encoding.
const HEX_RADIX: u32 = 16;

pub(crate) fn cursor_from_key(key: &[u8]) -> Bytes {
    Bytes::from(encode_hex(key))
}

pub(crate) fn decode_cursor(cursor: &[u8]) -> Option<Option<Bytes>> {
    if cursor == b"0" || cursor.is_empty() {
        return Some(None);
    }
    let text = std::str::from_utf8(cursor).ok()?;
    if text.len() % HEX_DIGITS_PER_BYTE != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(text.len() / HEX_DIGITS_PER_BYTE);
    let mut index = 0;
    while index < text.len() {
        let pair = text.get(index..index + HEX_DIGITS_PER_BYTE)?;
        bytes.push(u8::from_str_radix(pair, HEX_RADIX).ok()?);
        index += HEX_DIGITS_PER_BYTE;
    }
    Some(Some(Bytes::from(bytes)))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * HEX_DIGITS_PER_BYTE);
    for byte in bytes {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
}
