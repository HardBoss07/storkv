//! Command argument parsing and error mapping helpers.

use bytes::Bytes;
use storkv_core::{StorkvError, TimestampMs};

use super::super::codec::Resp;
use super::super::commands::MILLIS_PER_SECOND;

pub(crate) struct SetOptions {
    /// Absolute Unix epoch milliseconds, or zero when the value has no expiry.
    pub expires_at_ms: Option<u64>,
    pub only_if_absent: bool,
    pub only_if_present: bool,
    pub return_previous: bool,
    pub keep_ttl: bool,
}

pub(crate) fn parse_set_options(args: &[Bytes]) -> Result<SetOptions, Resp> {
    let mut options = SetOptions {
        expires_at_ms: None,
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
                let Some(seconds) = positive_integer(args.get(index + 1)) else {
                    return Err(Resp::error("ERR invalid expire time in 'set' command"));
                };
                set_expiry(&mut options, expiry_after(seconds, MILLIS_PER_SECOND))?;
                index += 2;
            }
            "PX" => {
                let Some(millis) = positive_integer(args.get(index + 1)) else {
                    return Err(Resp::error("ERR invalid expire time in 'set' command"));
                };
                set_expiry(&mut options, expiry_after(millis, 1))?;
                index += 2;
            }
            "EXAT" => {
                let Some(seconds) = positive_integer(args.get(index + 1)) else {
                    return Err(Resp::error("ERR invalid expire time in 'set' command"));
                };
                set_expiry(&mut options, seconds.saturating_mul(MILLIS_PER_SECOND))?;
                index += 2;
            }
            "PXAT" => {
                let Some(millis) = positive_integer(args.get(index + 1)) else {
                    return Err(Resp::error("ERR invalid expire time in 'set' command"));
                };
                set_expiry(&mut options, millis)?;
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
    if options.keep_ttl && options.expires_at_ms.is_some() {
        return Err(Resp::error("ERR syntax error"));
    }
    Ok(options)
}

fn set_expiry(options: &mut SetOptions, expires_at_ms: u64) -> Result<(), Resp> {
    if options.expires_at_ms.replace(expires_at_ms).is_some() {
        return Err(Resp::error("ERR syntax error"));
    }
    Ok(())
}

fn positive_integer(value: Option<&Bytes>) -> Option<u64> {
    let parsed = value.and_then(|arg| parse_i64(arg))?;
    if parsed <= 0 {
        return None;
    }
    Some(parsed as u64)
}

fn expiry_after(amount: u64, scale: u64) -> u64 {
    TimestampMs::now()
        .get()
        .saturating_add(amount.saturating_mul(scale))
}

/// Expiry update requested by `GETEX`, or `None` when it has no option.
#[derive(Clone, Copy)]
pub(crate) enum GetExMode {
    /// Remove the expiry.
    Persist,
    /// Set an absolute expiry in Unix epoch milliseconds.
    ExpireAt(u64),
}

pub(crate) fn parse_getex_options(args: &[Bytes]) -> Result<Option<GetExMode>, Resp> {
    if args.len() <= 2 {
        return Ok(None);
    }
    let option = upper(&args[2]);
    if option == "PERSIST" {
        if args.len() != 3 {
            return Err(Resp::error("ERR syntax error"));
        }
        return Ok(Some(GetExMode::Persist));
    }
    if args.len() != 4 {
        return Err(Resp::error("ERR syntax error"));
    }
    let Some(value) = args.get(3).and_then(|arg| parse_i64(arg)) else {
        return Err(Resp::error("ERR value is not an integer or out of range"));
    };
    if value <= 0 {
        return Err(Resp::error("ERR invalid expire time in 'getex' command"));
    }
    let value = value as u64;
    let mode = match option.as_str() {
        "EX" => GetExMode::ExpireAt(expiry_after(value, MILLIS_PER_SECOND)),
        "PX" => GetExMode::ExpireAt(expiry_after(value, 1)),
        "EXAT" => GetExMode::ExpireAt(value.saturating_mul(MILLIS_PER_SECOND)),
        "PXAT" => GetExMode::ExpireAt(value),
        _ => return Err(Resp::error("ERR syntax error")),
    };
    Ok(Some(mode))
}

/// Conditional modifiers accepted by `EXPIREAT` and `PEXPIREAT`.
#[derive(Clone, Copy, Default)]
pub(crate) struct ExpireConditions {
    nx: bool,
    xx: bool,
    gt: bool,
    lt: bool,
}

pub(crate) fn parse_expire_conditions(
    args: &[Bytes],
    start: usize,
) -> Result<ExpireConditions, Resp> {
    let mut conditions = ExpireConditions::default();
    for arg in &args[start..] {
        match upper(arg).as_str() {
            "NX" => conditions.nx = true,
            "XX" => conditions.xx = true,
            "GT" => conditions.gt = true,
            "LT" => conditions.lt = true,
            _ => return Err(Resp::error("ERR Unsupported option")),
        }
    }
    if conditions.nx && (conditions.xx || conditions.gt || conditions.lt) {
        return Err(Resp::error(
            "ERR NX and XX, GT or LT options at the same time are not compatible",
        ));
    }
    if conditions.gt && conditions.lt {
        return Err(Resp::error(
            "ERR GT and LT options at the same time are not compatible",
        ));
    }
    Ok(conditions)
}

impl ExpireConditions {
    /// Return `true` when the new absolute expiry satisfies every condition.
    ///
    /// A `current` value of zero means the key has no expiry (unlimited).
    pub(crate) fn matches(&self, current: u64, target: u64) -> bool {
        let nx = !self.nx || current == 0;
        let xx = !self.xx || current != 0;
        let gt = !self.gt || (current != 0 && target > current);
        let lt = !self.lt || (current == 0 || target < current);
        nx && xx && gt && lt
    }
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
