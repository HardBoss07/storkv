//! StorKV wire protocol: length-prefixed frames carrying typed requests,
//! responses, telemetry, and errors over TCP or Unix-domain sockets.
//!
//! The codec has no I/O and no runtime dependency. Callers feed a
//! [`bytes::BytesMut`] buffer to [`Message::decode`] and receive one complete
//! message at a time. [`Message::encode`] writes a frame that any peer can
//! decode, including peers written in another language.
//!
//! # Frame layout
//!
//! ```text
//! +----------------+-----------+-----------+-----------------+--------+
//! | length: u32 BE | magic: 8B | version:2B| request_id: u64 | tag:1B |
//! +----------------+-----------+-----------+-----------------+--------+
//! | payload ...                                                       |
//! +-------------------------------------------------------------------+
//! ```
//!
//! `magic` is [`PROTOCOL_MAGIC`], `version` is [`PROTOCOL_VERSION`], and the
//! payload is either a [`Request`] or a [`Response`]. The length prefix counts
//! the bytes after itself and must not exceed [`MAX_FRAME_SIZE`].
//!
//! # Example
//!
//! ```
//! use storkv_proto::{Message, MessagePayload, Request, Response};
//!
//! let message = Message::new_request(7, Request::Ping);
//! let encoded = message.encode();
//! let mut buffer = bytes::BytesMut::from(&encoded[..]);
//! let decoded = Message::decode(&mut buffer).expect("complete frame");
//! assert_eq!(
//!     decoded,
//!     Some(Message::new_request(7, Request::Ping))
//! );
//! # let _: MessagePayload = MessagePayload::Response(Response::Pong);
//! ```

#![warn(missing_docs)]

mod endpoint;
mod error;
mod frame;
mod request;
mod response;

pub use endpoint::{is_unix_socket_addr, unix_socket_path};
pub use error::StorKvWireError;
pub use frame::{MAX_FRAME_SIZE, Message, MessagePayload, PROTOCOL_MAGIC, PROTOCOL_VERSION};
pub use request::{
    Request, WireCompactionStyle, WireDurability, WireMergeOp, WireRetention, WireScopeOptions,
    WireTxMutation,
};
pub use response::{
    Response, WireAdminStats, WireBackupEntry, WireBackupKind, WireBackupReport, WireBackupStatus,
    WireBackupVerifyError, WireBackupVerifyReport, WireChangeEvent, WireKeyMeta, WireLevelStat,
    WireScopeHandle, WireScopeStat, WireTelemetrySnapshot, WireTransportStat,
};
