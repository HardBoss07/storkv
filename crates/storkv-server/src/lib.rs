//! StorKV network server: TCP and Unix-domain socket transports, request
//! dispatch, per-scope metrics, and watch subscriptions.

mod config;
mod dispatch;
mod listener;
mod metrics;
mod server;

pub use config::ServerConfig;
pub use dispatch::core_error_to_wire;
pub use listener::{AcceptedStream, ServerListener};
pub use server::Server;
