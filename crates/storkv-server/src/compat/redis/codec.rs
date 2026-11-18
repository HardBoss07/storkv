//! RESP2/RESP3 value model shared by the decoder and encoder.
//!
//! Parsing lives in [`codec_decode`] and rendering in [`codec_encode`].

use bytes::Bytes;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Resp {
    Simple(Bytes),
    Error(Bytes),
    Integer(i64),
    Bulk(Option<Bytes>),
    Array(Option<Vec<Resp>>),
    Boolean(bool),
    Double(f64),
    Null,
    Map(Vec<(Resp, Resp)>),
    Set(Vec<Resp>),
    Push(Vec<Resp>),
    Verbatim(Bytes),
    BigNumber(Bytes),
}

impl Resp {
    pub(crate) fn simple(text: impl Into<Bytes>) -> Self {
        Self::Simple(text.into())
    }

    pub(crate) fn error(text: impl Into<Bytes>) -> Self {
        Self::Error(text.into())
    }

    pub(crate) fn bulk(value: impl Into<Bytes>) -> Self {
        Self::Bulk(Some(value.into()))
    }

    pub(crate) fn null() -> Self {
        Self::Bulk(None)
    }

    pub(crate) fn array(values: Vec<Resp>) -> Self {
        Self::Array(Some(values))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParseError {
    Protocol(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protocol(message) => formatter.write_str(message),
        }
    }
}
