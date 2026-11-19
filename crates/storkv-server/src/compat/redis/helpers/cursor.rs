//! Cursor encoding for Redis `SCAN`.

use bytes::Bytes;

pub(crate) fn cursor_from_key(key: &[u8]) -> Bytes {
    Bytes::from(encode_hex(key))
}

pub(crate) fn decode_cursor(cursor: &[u8]) -> Option<Option<Bytes>> {
    if cursor == b"0" || cursor.is_empty() {
        return Some(None);
    }
    let text = std::str::from_utf8(cursor).ok()?;
    if text.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(text.len() / 2);
    let mut index = 0;
    while index < text.len() {
        let pair = text.get(index..index + 2)?;
        bytes.push(u8::from_str_radix(pair, 16).ok()?);
        index += 2;
    }
    Some(Some(Bytes::from(bytes)))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
}
