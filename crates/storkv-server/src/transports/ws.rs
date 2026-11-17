//! WebSocket transport: session loop and JSON-RPC framing.

mod json_rpc;
mod session;

pub use session::standalone_router;
pub(crate) use session::upgrade;
