//! Shared parsing, error mapping, glob matching, and cursor helpers.

use std::time::Duration;

use bytes::Bytes;
use storkv_core::StorKvError;

use super::codec::Resp;

pub(super) struct SetOptions {
    pub ttl: Option<Duration>,
    pub only_if_absent: bool,
    pub only_if_present: bool,
    pub return_previous: bool,
    pub keep_ttl: bool,
}

pub(super) fn parse_set_options(args: &[Bytes]) -> Result<SetOptions, Resp> {
    let mut options = SetOptions {
        ttl: None,
        only_if_absent: false,
        only_if_present: false,
        return_previous: false,
        keep_ttl: false,
    };
    let mut index = 3;
    while index < args.len() {
        let option = upper(&args[index]);
        match option.as_str() {
            "EX" => {
                let Some(seconds) = args.get(index + 1).and_then(|arg| parse_i64(arg)) else {
                    return Err(Resp::error("ERR invalid expire time in 'set' command"));
                };
                if seconds <= 0 {
                    return Err(Resp::error("ERR invalid expire time in 'set' command"));
                }
                options.ttl = Some(Duration::from_secs(seconds as u64));
                index += 2;
            }
            "PX" => {
                let Some(millis) = args.get(index + 1).and_then(|arg| parse_i64(arg)) else {
                    return Err(Resp::error("ERR invalid expire time in 'set' command"));
                };
                if millis <= 0 {
                    return Err(Resp::error("ERR invalid expire time in 'set' command"));
                }
                options.ttl = Some(Duration::from_millis(millis as u64));
                index += 2;
            }
            "NX" => {
                options.only_if_absent = true;
                index += 1;
            }
            "XX" => {
                options.only_if_present = true;
                index += 1;
            }
            "GET" => {
                options.return_previous = true;
                index += 1;
            }
            "KEEPTTL" => {
                options.keep_ttl = true;
                index += 1;
            }
            other => {
                return Err(Resp::error(format!(
                    "ERR syntax error near '{other}' in 'set' command"
                )));
            }
        }
    }
    if options.only_if_absent && options.only_if_present {
        return Err(Resp::error("ERR syntax error"));
    }
    Ok(options)
}

pub(super) fn is_known_command(command: &str) -> bool {
    matches!(
        command,
        "PING"
            | "ECHO"
            | "QUIT"
            | "HELLO"
            | "SELECT"
            | "INFO"
            | "DBSIZE"
            | "FLUSHDB"
            | "FLUSHALL"
            | "GET"
            | "SET"
            | "SETEX"
            | "PSETEX"
            | "SETNX"
            | "MGET"
            | "MSET"
            | "DEL"
            | "UNLINK"
            | "EXISTS"
            | "INCR"
            | "DECR"
            | "INCRBY"
            | "DECRBY"
            | "APPEND"
            | "STRLEN"
            | "EXPIRE"
            | "PEXPIRE"
            | "PERSIST"
            | "TTL"
            | "PTTL"
            | "KEYS"
            | "SCAN"
            | "MULTI"
            | "EXEC"
            | "DISCARD"
            | "WATCH"
            | "UNWATCH"
    )
}

pub(super) fn upper(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_ascii_uppercase()
}

pub(super) fn parse_u64(bytes: &[u8]) -> Option<u64> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

pub(super) fn parse_i64(bytes: &[u8]) -> Option<i64> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

pub(super) fn wrong_arity(command: &str) -> Resp {
    Resp::error(format!(
        "ERR wrong number of arguments for '{}' command",
        command
    ))
}

pub(super) fn storage_error(error: StorKvError) -> Resp {
    match error {
        StorKvError::InvalidOperation(message) => Resp::error(format!("ERR {message}")),
        StorKvError::ValueTooLarge { .. } => Resp::error("ERR value too large"),
        StorKvError::KeyTooLarge { .. } => Resp::error("ERR key too large"),
        StorKvError::WriteConflict { .. } => Resp::error("ERR write conflict, retry"),
        other => Resp::error(format!("ERR {other}")),
    }
}

pub(super) fn cursor_from_key(key: &[u8]) -> Bytes {
    Bytes::from(encode_hex(key))
}

pub(super) fn decode_cursor(cursor: &[u8]) -> Option<Option<Bytes>> {
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

/// Glob matcher supporting `*`, `?`, and `[abc]`/`[a-z]` classes.
pub(super) fn glob_match(pattern: &[u8], text: &[u8]) -> bool {
    glob_match_at(pattern, text)
}

fn glob_match_at(pattern: &[u8], text: &[u8]) -> bool {
    let Some((&first, rest)) = pattern.split_first() else {
        return text.is_empty();
    };
    match first {
        b'*' => {
            let mut remaining = rest;
            while remaining.first() == Some(&b'*') {
                remaining = &remaining[1..];
            }
            if remaining.is_empty() {
                return true;
            }
            for index in 0..=text.len() {
                if glob_match_at(remaining, &text[index..]) {
                    return true;
                }
            }
            false
        }
        b'?' => text
            .split_first()
            .is_some_and(|(_, tail)| glob_match_at(rest, tail)),
        b'[' => {
            let Some((matched, after)) = match_class(pattern, text) else {
                return false;
            };
            matched && glob_match_at(after, &text[1..])
        }
        b'\\' => match rest.split_first() {
            Some((&escaped, tail)) => match text.split_first() {
                Some((&actual, remaining)) if actual == escaped => glob_match_at(tail, remaining),
                _ => false,
            },
            None => text.first() == Some(&b'\\') && text.len() == 1,
        },
        literal => match text.split_first() {
            Some((&actual, remaining)) if actual == literal => glob_match_at(rest, remaining),
            _ => false,
        },
    }
}

fn match_class<'a>(pattern: &'a [u8], text: &[u8]) -> Option<(bool, &'a [u8])> {
    let target = *text.first()?;
    let mut index = 1;
    let mut matched = false;
    let negated = pattern.get(index) == Some(&b'^');
    if negated {
        index += 1;
    }
    let mut closed = false;
    while index < pattern.len() {
        let byte = pattern[index];
        if byte == b']' && index > 1 {
            closed = true;
            index += 1;
            break;
        }
        if pattern.get(index + 1) == Some(&b'-') {
            if let Some(end) = pattern.get(index + 2) {
                if target >= byte && target <= *end {
                    matched = true;
                }
                index += 3;
                continue;
            }
        }
        if target == byte {
            matched = true;
        }
        index += 1;
    }
    if !closed {
        return None;
    }
    Some((matched != negated, pattern.get(index..).unwrap_or(&[])))
}
