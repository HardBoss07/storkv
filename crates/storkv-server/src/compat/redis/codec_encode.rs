//! RESP2/RESP3 encoder.
//!
//! Renders the shared value model in RESP2 or RESP3 depending on the negotiated
//! `HELLO` version.

use bytes::{Bytes, BytesMut};

use super::codec::Resp;

/// Append one value to `destination` using the requested RESP version.
pub(crate) fn encode(value: &Resp, version: u8, destination: &mut BytesMut) {
    match value {
        Resp::Simple(text) => {
            destination.extend_from_slice(b"+");
            destination.extend_from_slice(text);
            destination.extend_from_slice(b"\r\n");
        }
        Resp::Error(text) => {
            destination.extend_from_slice(b"-");
            destination.extend_from_slice(text);
            destination.extend_from_slice(b"\r\n");
        }
        Resp::Integer(number) => {
            destination.extend_from_slice(b":");
            destination.extend_from_slice(number.to_string().as_bytes());
            destination.extend_from_slice(b"\r\n");
        }
        Resp::Bulk(Some(bytes)) => {
            destination.extend_from_slice(b"$");
            destination.extend_from_slice(bytes.len().to_string().as_bytes());
            destination.extend_from_slice(b"\r\n");
            destination.extend_from_slice(bytes);
            destination.extend_from_slice(b"\r\n");
        }
        Resp::Bulk(None) | Resp::Null => {
            if version >= 3 {
                destination.extend_from_slice(b"_\r\n");
            } else {
                destination.extend_from_slice(b"$-1\r\n");
            }
        }
        Resp::Boolean(flag) => {
            if version >= 3 {
                destination.extend_from_slice(if *flag { b"#t\r\n" } else { b"#f\r\n" });
            } else {
                destination.extend_from_slice(if *flag { b":1\r\n" } else { b":0\r\n" });
            }
        }
        Resp::Double(number) => {
            if version >= 3 {
                destination.extend_from_slice(b",");
                destination.extend_from_slice(format_double(*number).as_bytes());
                destination.extend_from_slice(b"\r\n");
            } else {
                let text = Bytes::from(format_double(*number));
                encode(&Resp::Bulk(Some(text)), version, destination);
            }
        }
        Resp::Verbatim(bytes) => {
            if version >= 3 {
                destination.extend_from_slice(b"=");
                destination.extend_from_slice(bytes.len().to_string().as_bytes());
                destination.extend_from_slice(b"\r\n");
                destination.extend_from_slice(bytes);
                destination.extend_from_slice(b"\r\n");
            } else {
                encode(&Resp::Bulk(Some(bytes.clone())), version, destination);
            }
        }
        Resp::BigNumber(bytes) => {
            if version >= 3 {
                destination.extend_from_slice(b"(");
                destination.extend_from_slice(bytes);
                destination.extend_from_slice(b"\r\n");
            } else {
                encode(&Resp::Bulk(Some(bytes.clone())), version, destination);
            }
        }
        Resp::Array(values) => encode_sequence(b'*', values.as_deref(), version, destination),
        Resp::Set(values) => {
            if version >= 3 {
                encode_sequence(b'~', Some(values), version, destination);
            } else {
                encode_sequence(b'*', Some(values), version, destination);
            }
        }
        Resp::Push(values) => {
            if version >= 3 {
                encode_sequence(b'>', Some(values), version, destination);
            } else {
                encode_sequence(b'*', Some(values), version, destination);
            }
        }
        Resp::Map(pairs) => encode_map(pairs, version, destination),
    }
}

fn encode_map(pairs: &[(Resp, Resp)], version: u8, destination: &mut BytesMut) {
    if version >= 3 {
        destination.extend_from_slice(b"%");
        destination.extend_from_slice(pairs.len().to_string().as_bytes());
        destination.extend_from_slice(b"\r\n");
    } else {
        destination.extend_from_slice(b"*");
        destination.extend_from_slice((pairs.len() * 2).to_string().as_bytes());
        destination.extend_from_slice(b"\r\n");
    }
    for (key, value) in pairs {
        encode(key, version, destination);
        encode(value, version, destination);
    }
}

fn encode_sequence(prefix: u8, values: Option<&[Resp]>, version: u8, destination: &mut BytesMut) {
    match values {
        Some(values) => {
            destination.extend_from_slice(&[prefix]);
            destination.extend_from_slice(values.len().to_string().as_bytes());
            destination.extend_from_slice(b"\r\n");
            for value in values {
                encode(value, version, destination);
            }
        }
        None => destination.extend_from_slice(b"*-1\r\n"),
    }
}

fn format_double(number: f64) -> String {
    if number.is_infinite() {
        if number.is_sign_negative() {
            "-inf".into()
        } else {
            "inf".into()
        }
    } else if number.is_nan() {
        "nan".into()
    } else {
        number.to_string()
    }
}
