//! StorKV wire protocol: length-prefixed frames carrying typed requests,
//! responses, telemetry, and errors over TCP or Unix-domain sockets.

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
    Response, WireAdminStats, WireChangeEvent, WireLevelStat, WireScopeHandle, WireScopeStat,
    WireTelemetrySnapshot,
};
