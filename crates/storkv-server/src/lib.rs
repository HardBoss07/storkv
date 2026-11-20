//! StorKV network server: TCP and Unix-domain socket transports, request
//! dispatch, per-scope metrics, and watch subscriptions.
//!
//! [`Server`] wraps one [`storkv_core::Db`] and dispatches native protocol
//! requests without knowledge of the transport. The [`transports`] module adds
//! HTTP/JSON, WebSocket, gRPC, Redis, Memcached, and etcd-compatible front
//! ends. Each front end translates its protocol into a [`storkv_proto::Request`]
//! and calls the same dispatcher, so all transports share one code path and one
//! set of consistency rules.
//!
//! Enable strict admin mode with [`Server::with_strict_admin`] to reject
//! administrative requests unless the connection is an admin connection.

#![warn(missing_docs)]

pub mod compat;
mod config;
mod dispatch;
mod listener;
mod metrics;
mod replication;
mod server;
pub mod tls;
pub mod transports;

pub use config::{ReplicationRole, ServerConfig};
pub use dispatch::core_error_to_wire;
pub use listener::{AcceptedStream, ServerListener};
pub use server::Server;
pub use transports::{TransportKind, TransportOptions};
