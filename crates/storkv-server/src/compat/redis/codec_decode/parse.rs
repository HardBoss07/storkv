//! RESP2/RESP3 top-level parser and inline commands.

use bytes::{Buf, Bytes, BytesMut};

use super::super::codec::{ParseError, Resp};
use super::value::{CRLF_LEN, parse_value};

pub(crate) const MAX_BULK_LEN: usize = storkv_core::format::MAX_VAL_LEN;
pub(crate) const MAX_ARRAY_LEN: usize = 1_000_000;
pub(crate) const MAX_NESTING: usize = 32;
pub(crate) const MAX_INLINE_LINE_LEN: usize = 64 * 1024;
/// Extra space allowed for framing bytes beyond the largest bulk or inline line.
const READ_BUFFER_FRAMING_SLACK: usize = 1024;
pub(crate) const MAX_READ_BUFFER: usize =
    MAX_BULK_LEN + MAX_INLINE_LINE_LEN + READ_BUFFER_FRAMING_SLACK;

/// Parse one RESP value or inline command from the front of `buffer`.
///
/// `Ok(None)` means more bytes are required. On success the consumed bytes are
/// removed from `buffer`.
pub(crate) fn parse(buffer: &mut BytesMut) -> Result<Option<Resp>, ParseError> {
    let Some(first) = buffer.first().copied() else {
        return Ok(None);
    };
    let parsed = match first {
        b'+' | b'-' | b':' | b'$' | b'*' | b'_' | b'#' | b',' | b'%' | b'>' | b'~' | b'='
        | b'(' => parse_value(buffer, 0, 0)?,
        _ => parse_inline(buffer)?,
    };
    match parsed {
        Some((value, consumed)) => {
            buffer.advance(consumed);
            Ok(Some(value))
        }
        None => Ok(None),
    }
}

fn parse_inline(buffer: &[u8]) -> Result<Option<(Resp, usize)>, ParseError> {
    let Some(line_end) = super::value::find_crlf(buffer, 0) else {
        if buffer.len() > MAX_INLINE_LINE_LEN {
            return Err(ParseError::Protocol("inline command too long".into()));
        }
        return Ok(None);
    };
    if line_end > MAX_INLINE_LINE_LEN {
        return Err(ParseError::Protocol("inline command too long".into()));
    }
    let line = &buffer[..line_end];
    let arguments = split_inline(line)?;
    if arguments.is_empty() {
        return Err(ParseError::Protocol("empty inline command".into()));
    }
    let value = Resp::Array(Some(
        arguments
            .into_iter()
            .map(|argument| Resp::Bulk(Some(Bytes::copy_from_slice(&argument))))
            .collect(),
    ));
    Ok(Some((value, line_end + CRLF_LEN)))
}

fn split_inline(line: &[u8]) -> Result<Vec<Vec<u8>>, ParseError> {
    let mut arguments = Vec::new();
    let mut current = Vec::new();
    let mut index = 0;
    let mut quoted = false;
    while index < line.len() {
        let byte = line[index];
        match byte {
            b' ' | b'\t' if !quoted => {
                if !current.is_empty() {
                    arguments.push(std::mem::take(&mut current));
                }
            }
            b'"' => quoted = !quoted,
            b'\\' if quoted => {
                index += 1;
                if let Some(escaped) = line.get(index) {
                    current.push(*escaped);
                }
            }
            _ => current.push(byte),
        }
        index += 1;
    }
    if quoted {
        return Err(ParseError::Protocol("unbalanced quotes in request".into()));
    }
    if !current.is_empty() {
        arguments.push(current);
    }
    Ok(arguments)
}
