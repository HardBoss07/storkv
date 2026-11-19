//! RESP2/RESP3 parser.
//!
//! The parser accepts every RESP framing prefix (including inline commands for
//! `telnet`/`nc`) and enforces protocol limits before allocating.

mod args;
mod parse;
mod value;

pub(crate) use args::into_arguments;
pub(crate) use parse::{MAX_READ_BUFFER, parse};
