//! Command argument parsing and error mapping helpers.

use std::time::Duration;

use bytes::Bytes;
use storkv_core::StorkvError;

use super::super::codec::Resp;

pub(crate) struct SetOptions {
    pub ttl: Option<Duration>,
    pub only_if_absent: bool,
    pub only_if_present: bool,
    pub return_previous: bool,
    pub keep_ttl: bool,
}

pub(crate) fn parse_set_options(args: &[Bytes]) -> Result<SetOptions, Resp> {
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

pub(crate) fn is_known_command(command: &str) -> bool {
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

pub(crate) fn upper(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_ascii_uppercase()
}

pub(crate) fn parse_u64(bytes: &[u8]) -> Option<u64> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

pub(crate) fn parse_i64(bytes: &[u8]) -> Option<i64> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

pub(crate) fn wrong_arity(command: &str) -> Resp {
    Resp::error(format!(
        "ERR wrong number of arguments for '{}' command",
        command
    ))
}

pub(crate) fn readonly_error() -> Resp {
    Resp::error("READONLY You can't write against a read only replica.")
}

pub(crate) fn storage_error(error: StorkvError) -> Resp {
    match error {
        StorkvError::NotPrimary => readonly_error(),
        StorkvError::InvalidOperation(message) => Resp::error(format!("ERR {message}")),
        StorkvError::ValueTooLarge { .. } => Resp::error("ERR value too large"),
        StorkvError::KeyTooLarge { .. } => Resp::error("ERR key too large"),
        StorkvError::WriteConflict { .. } => Resp::error("ERR write conflict, retry"),
        other => Resp::error(format!("ERR {other}")),
    }
}
