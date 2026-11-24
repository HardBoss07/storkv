//! RESP2/RESP3 value parsing.

use bytes::Bytes;

use super::super::codec::{ParseError, Resp};
use super::parse::{MAX_ARRAY_LEN, MAX_BULK_LEN, MAX_NESTING};

type ArrayParse = Result<Option<(Option<Vec<Resp>>, usize)>, ParseError>;

const INITIAL_ARRAY_CAPACITY: usize = 1024;
/// Length of the `\r\n` line terminator in bytes.
pub(super) const CRLF_LEN: usize = 2;
/// Number of array elements that form one map entry.
const MAP_FIELDS_PER_PAIR: usize = 2;

pub(super) fn parse_value(
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
    let consumed_line = line_end + CRLF_LEN;
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
    let Some(terminator) = buffer.get(data_end..data_end.saturating_add(CRLF_LEN)) else {
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
    Ok(Some((value, data_end + CRLF_LEN)))
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
    let mut pairs = Vec::with_capacity(flattened.len() / MAP_FIELDS_PER_PAIR);
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
    let mut position = line_end + CRLF_LEN;
    if count < 0 {
        return Ok(Some((None, position)));
    }
    let count =
        usize::try_from(count).map_err(|_| ParseError::Protocol("array length overflow".into()))?;
    if count > MAX_ARRAY_LEN {
        return Err(ParseError::Protocol("array length exceeds limit".into()));
    }
    let mut values = Vec::with_capacity(count.min(INITIAL_ARRAY_CAPACITY));
    for _ in 0..count {
        let Some((value, consumed)) = parse_value(buffer, position, depth + 1)? else {
            return Ok(None);
        };
        values.push(value);
        position = consumed;
    }
    Ok(Some((Some(values), position)))
}

pub(super) fn find_crlf(buffer: &[u8], start: usize) -> Option<usize> {
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
