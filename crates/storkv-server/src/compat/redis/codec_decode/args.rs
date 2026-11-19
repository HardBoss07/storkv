//! Command argument extraction from parsed RESP values.

use bytes::Bytes;

use super::super::codec::{ParseError, Resp};

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
