//! Strict Memcached ASCII command-line parser.

use bytes::Bytes;

pub(crate) const MAX_KEY_LEN: usize = 250;
pub(crate) const MAX_VALUE_LEN: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StorageKind {
    Set,
    Add,
    Replace,
    Append,
    Prepend,
    Cas,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    Get {
        keys: Vec<Bytes>,
        with_cas: bool,
    },
    Storage {
        kind: StorageKind,
        key: Bytes,
        flags: u32,
        exptime: u32,
        bytes: usize,
        cas: Option<u64>,
        noreply: bool,
    },
    Delete {
        key: Bytes,
        noreply: bool,
    },
    Increment {
        key: Bytes,
        delta: u64,
        decrement: bool,
        noreply: bool,
    },
    Touch {
        key: Bytes,
        exptime: u32,
        noreply: bool,
    },
    FlushAll {
        delay: u32,
        noreply: bool,
    },
    Stats,
    Version,
    Quit,
}

impl Command {
    pub(crate) fn data_len(&self) -> Option<usize> {
        match self {
            Command::Storage { bytes, .. } => Some(*bytes),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParseError {
    pub(crate) message: String,
}

impl ParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub(crate) fn parse_line(line: &[u8]) -> Result<Command, ParseError> {
    let fields: Vec<&[u8]> = line
        .split(|byte| *byte == b' ')
        .filter(|field| !field.is_empty())
        .collect();
    let Some(command) = fields.first() else {
        return Err(ParseError::new("bad command line format"));
    };
    let command = String::from_utf8_lossy(command).to_ascii_lowercase();
    match command.as_str() {
        "get" | "gets" => {
            if fields.len() < 2 {
                return Err(ParseError::new("bad command line format"));
            }
            let keys = fields[1..]
                .iter()
                .map(|key| validate_key(key))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Command::Get {
                keys,
                with_cas: command == "gets",
            })
        }
        "set" | "add" | "replace" | "append" | "prepend" | "cas" => {
            let cas_command = command == "cas";
            let minimum = if cas_command { 6 } else { 5 };
            if fields.len() < minimum {
                return Err(ParseError::new("bad command line format"));
            }
            let key = validate_key(fields[1])?;
            let flags = parse_u32(fields[2])?;
            let exptime = parse_u32(fields[3])?;
            let bytes = parse_usize(fields[4])?;
            if bytes > MAX_VALUE_LEN {
                return Err(ParseError::new("object too large for cache"));
            }
            let (cas, noreply_index) = if cas_command {
                (Some(parse_u64(fields[5])?), 6)
            } else {
                (None, 5)
            };
            let noreply = fields
                .get(noreply_index)
                .is_some_and(|field| *field == b"noreply");
            if fields.len() > noreply_index + usize::from(noreply) {
                return Err(ParseError::new("bad command line format"));
            }
            let kind = match command.as_str() {
                "set" => StorageKind::Set,
                "add" => StorageKind::Add,
                "replace" => StorageKind::Replace,
                "append" => StorageKind::Append,
                "prepend" => StorageKind::Prepend,
                _ => StorageKind::Cas,
            };
            Ok(Command::Storage {
                kind,
                key,
                flags,
                exptime,
                bytes,
                cas,
                noreply,
            })
        }
        "delete" => {
            if fields.len() < 2 {
                return Err(ParseError::new("bad command line format"));
            }
            let key = validate_key(fields[1])?;
            let noreply = fields.len() == 3 && fields[2] == b"noreply";
            if fields.len() > 3 || (fields.len() == 3 && !noreply) {
                return Err(ParseError::new("bad command line format"));
            }
            Ok(Command::Delete { key, noreply })
        }
        "incr" | "decr" => {
            if fields.len() < 3 {
                return Err(ParseError::new("bad command line format"));
            }
            let key = validate_key(fields[1])?;
            let delta = parse_u64(fields[2])?;
            let noreply = fields.len() == 4 && fields[3] == b"noreply";
            if fields.len() > 4 || (fields.len() == 4 && !noreply) {
                return Err(ParseError::new("bad command line format"));
            }
            Ok(Command::Increment {
                key,
                delta,
                decrement: command == "decr",
                noreply,
            })
        }
        "touch" => {
            if fields.len() < 3 {
                return Err(ParseError::new("bad command line format"));
            }
            let key = validate_key(fields[1])?;
            let exptime = parse_u32(fields[2])?;
            let noreply = fields.len() == 4 && fields[3] == b"noreply";
            if fields.len() > 4 || (fields.len() == 4 && !noreply) {
                return Err(ParseError::new("bad command line format"));
            }
            Ok(Command::Touch {
                key,
                exptime,
                noreply,
            })
        }
        "flush_all" => {
            let mut delay = 0;
            let mut noreply = false;
            for field in &fields[1..] {
                if *field == b"noreply" {
                    noreply = true;
                } else {
                    delay = parse_u32(field)?;
                }
            }
            Ok(Command::FlushAll { delay, noreply })
        }
        "stats" => Ok(Command::Stats),
        "version" => Ok(Command::Version),
        "quit" => Ok(Command::Quit),
        _ => Err(ParseError::new("unknown command")),
    }
}

fn validate_key(field: &[u8]) -> Result<Bytes, ParseError> {
    if field.is_empty() || field.len() > MAX_KEY_LEN {
        return Err(ParseError::new("bad command line format"));
    }
    Ok(Bytes::copy_from_slice(field))
}

fn parse_u32(field: &[u8]) -> Result<u32, ParseError> {
    std::str::from_utf8(field)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| ParseError::new("bad command line format"))
}

fn parse_u64(field: &[u8]) -> Result<u64, ParseError> {
    std::str::from_utf8(field)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| ParseError::new("bad command line format"))
}

fn parse_usize(field: &[u8]) -> Result<usize, ParseError> {
    std::str::from_utf8(field)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| ParseError::new("bad command line format"))
}
