//! RESP2/RESP3 parser.
//!
//! The parser accepts every RESP framing prefix (including inline commands for
//! `telnet`/`nc`) and enforces protocol limits before allocating.

use bytes::{Buf, Bytes, BytesMut};

use super::codec::{ParseError, Resp};

type ArrayParse = Result<Option<(Option<Vec<Resp>>, usize)>, ParseError>;

pub(crate) const MAX_BULK_LEN: usize = 512 * 1024 * 1024;
pub(crate) const MAX_ARRAY_LEN: usize = 1_000_000;
pub(crate) const MAX_NESTING: usize = 32;

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
    let Some(line_end) = find_crlf(buffer, 0) else {
        return Ok(None);
    };
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
    Ok(Some((value, line_end + 2)))
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

fn parse_value(
    buffer: &[u8],
    position: usize,
    depth: usize,
) -> Result<Option<(Resp, usize)>, ParseError> {
    if depth > MAX_NESTING {
        return Err(ParseError::Protocol("nesting depth exceeded".into()));
    }
    let Some(prefix) = buffer.get(position).copied() else {
        return Ok(None);
    };
    let line_start = position + 1;
    let Some(line_end) = find_crlf(buffer, line_start) else {
        return Ok(None);
    };
    let line = &buffer[line_start..line_end];
    let consumed_line = line_end + 2;
    let count_line = line_start;
    match prefix {
        b'+' => Ok(Some((Resp::Simple(bytes_from(line)), consumed_line))),
        b'-' => Ok(Some((Resp::Error(bytes_from(line)), consumed_line))),
        b':' => {
            let value = parse_i64(line)?;
            Ok(Some((Resp::Integer(value), consumed_line)))
        }
        b'(' => Ok(Some((Resp::BigNumber(bytes_from(line)), consumed_line))),
        b',' => {
            let text = std::str::from_utf8(line)
                .map_err(|_| ParseError::Protocol("invalid double".into()))?;
            let value = text
                .parse::<f64>()
                .map_err(|_| ParseError::Protocol("invalid double".into()))?;
            Ok(Some((Resp::Double(value), consumed_line)))
        }
        b'#' => match line {
            b"t" => Ok(Some((Resp::Boolean(true), consumed_line))),
            b"f" => Ok(Some((Resp::Boolean(false), consumed_line))),
            _ => Err(ParseError::Protocol("invalid boolean".into())),
        },
        b'_' => Ok(Some((Resp::Null, consumed_line))),
        b'$' | b'=' => parse_bulk(buffer, line, prefix, consumed_line),
        b'*' | b'>' | b'~' => {
            let Some((values, consumed)) = parse_array(buffer, count_line, depth)? else {
                return Ok(None);
            };
            let value = match prefix {
                b'*' => Resp::Array(values),
                b'>' => Resp::Push(values.unwrap_or_default()),
                _ => Resp::Set(values.unwrap_or_default()),
            };
            Ok(Some((value, consumed)))
        }
        b'%' => parse_map(buffer, count_line, depth),
        _ => Err(ParseError::Protocol(format!(
            "unknown RESP prefix: {}",
            char::from(prefix)
        ))),
    }
}

fn parse_bulk(
    buffer: &[u8],
    line: &[u8],
    prefix: u8,
    consumed_line: usize,
) -> Result<Option<(Resp, usize)>, ParseError> {
    let length = parse_i64(line)?;
    if length < 0 {
        return Ok(Some((Resp::Bulk(None), consumed_line)));
    }
    let length =
        usize::try_from(length).map_err(|_| ParseError::Protocol("bulk length overflow".into()))?;
    if length > MAX_BULK_LEN {
        return Err(ParseError::Protocol("bulk length exceeds limit".into()));
    }
    let Some(data_end) = consumed_line.checked_add(length) else {
        return Err(ParseError::Protocol("bulk length overflow".into()));
    };
    let Some(body) = buffer.get(consumed_line..data_end) else {
        return Ok(None);
    };
    let Some(terminator) = buffer.get(data_end..data_end.saturating_add(2)) else {
        return Ok(None);
    };
    if terminator != b"\r\n" {
        return Err(ParseError::Protocol("invalid bulk terminator".into()));
    }
    let value = bytes_from(body);
    let value = if prefix == b'=' {
        Resp::Verbatim(value)
    } else {
        Resp::Bulk(Some(value))
    };
    Ok(Some((value, data_end + 2)))
}

fn parse_map(
    buffer: &[u8],
    count_line: usize,
    depth: usize,
) -> Result<Option<(Resp, usize)>, ParseError> {
    let Some((values, consumed)) = parse_array(buffer, count_line, depth)? else {
        return Ok(None);
    };
    let flattened = values.unwrap_or_default();
    let mut pairs = Vec::with_capacity(flattened.len() / 2);
    let mut iterator = flattened.into_iter();
    while let Some(key) = iterator.next() {
        let Some(value) = iterator.next() else {
            return Err(ParseError::Protocol("odd map length".into()));
        };
        pairs.push((key, value));
    }
    Ok(Some((Resp::Map(pairs), consumed)))
}

fn parse_array(buffer: &[u8], start: usize, depth: usize) -> ArrayParse {
    let Some(line_end) = find_crlf(buffer, start) else {
        return Ok(None);
    };
    let count = parse_i64(&buffer[start..line_end])?;
    let mut position = line_end + 2;
    if count < 0 {
        return Ok(Some((None, position)));
    }
    let count =
        usize::try_from(count).map_err(|_| ParseError::Protocol("array length overflow".into()))?;
    if count > MAX_ARRAY_LEN {
        return Err(ParseError::Protocol("array length exceeds limit".into()));
    }
    let mut values = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        let Some((value, consumed)) = parse_value(buffer, position, depth + 1)? else {
            return Ok(None);
        };
        values.push(value);
        position = consumed;
    }
    Ok(Some((Some(values), position)))
}

fn find_crlf(buffer: &[u8], start: usize) -> Option<usize> {
    let mut index = start;
    while index + 1 < buffer.len() {
        if buffer[index] == b'\r' && buffer[index + 1] == b'\n' {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn parse_i64(line: &[u8]) -> Result<i64, ParseError> {
    let text = std::str::from_utf8(line)
        .map_err(|_| ParseError::Protocol("invalid integer encoding".into()))?;
    text.parse::<i64>()
        .map_err(|_| ParseError::Protocol("invalid integer".into()))
}

fn bytes_from(bytes: &[u8]) -> Bytes {
    Bytes::copy_from_slice(bytes)
}

/// Convert a parsed array into command arguments.
pub(crate) fn into_arguments(value: Resp) -> Result<Vec<Bytes>, ParseError> {
    match value {
        Resp::Array(Some(values)) => values.into_iter().map(argument).collect(),
        Resp::Array(None) => Err(ParseError::Protocol("null command array".into())),
        Resp::Push(values) | Resp::Set(values) => values.into_iter().map(argument).collect(),
        _ => Err(ParseError::Protocol(
            "expected an array of bulk strings".into(),
        )),
    }
}

fn argument(value: Resp) -> Result<Bytes, ParseError> {
    match value {
        Resp::Bulk(Some(bytes)) | Resp::Simple(bytes) | Resp::Verbatim(bytes) => Ok(bytes),
        Resp::Integer(number) => Ok(Bytes::from(number.to_string())),
        _ => Err(ParseError::Protocol(
            "command arguments must be strings".into(),
        )),
    }
}
