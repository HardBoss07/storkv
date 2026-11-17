//! Async and blocking clients for the StorKV wire protocol.
//!
//! [`AsyncClient`] connects over native TCP or Unix-domain sockets, HTTP,
//! WebSocket, or gRPC. [`AsyncClient::connect_uri`] selects the transport from
//! the URI scheme. [`SyncClient`] wraps the async client for blocking callers.
//!
//! # Example
//!
//! ```no_run
//! use storkv_client::AsyncClient;
//!
//! # async fn example() -> storkv_client::ClientResult<()> {
//! let client = AsyncClient::connect_uri("storkv://127.0.0.1:7379").await?;
//! client.ping().await?;
//! let scope = client
//!     .create_scope(
//!         "default",
//!         storkv_proto::WireDurability::StrictFsync,
//!         storkv_proto::WireRetention::Permanent,
//!         None,
//!     )
//!     .await?;
//! client.put(&scope, b"key", b"value").await?;
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

mod async_client;
mod backend;
pub mod grpc;
mod sync_client;
mod transaction;
mod transport;
mod uri;

pub use async_client::{AsyncClient, WatchStream};
pub use sync_client::SyncClient;
pub use transaction::AsyncTransaction;
pub use transport::{ClientError, ClientResult, ClientStream, ScopeHandle};
pub use uri::TransportKind;
