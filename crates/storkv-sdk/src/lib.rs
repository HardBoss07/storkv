//! Unified SDK for StorKV.
//!
//! [`StorkvPool`] gives one pooled API for embedded storage and network
//! servers. A `file://` or `embedded://` URI uses `storkv-core` in the current
//! process. A `storkv://`, `grpc://`, `http://`, `https://`, `ws://`, `wss://`,
//! or `unix://` URI uses pooled `storkv-client` connections. A bare `host:port`
//! value selects the native transport.
//!
//! # Quick start
//!
//! This example opens an embedded database, creates a scope, writes a value,
//! and runs one transaction:
//!
//! ```rust
//! use storkv_sdk::{ScopeDefinition, StorkvPoolOptions};
//!
//! #[tokio::main]
//! async fn main() -> storkv_sdk::Result<()> {
//!     let nanos = std::time::SystemTime::now()
//!         .duration_since(std::time::UNIX_EPOCH)
//!         .map(|elapsed| elapsed.as_nanos())
//!         .unwrap_or(0);
//!     let path = std::env::temp_dir().join(format!(
//!         "storkv-sdk-doc-{}-{nanos}",
//!         std::process::id()
//!     ));
//!     std::fs::create_dir_all(&path).ok();
//!     let uri = format!("file://{}", path.display());
//!
//!     let pool = StorkvPoolOptions::new()
//!         .max_connections(4)
//!         .connect(&uri)
//!         .await?;
//!     pool.provision_scopes(&[ScopeDefinition::new("users")]).await?;
//!
//!     let users = pool.scope("users");
//!     users.put("alice", b"active").await?;
//!     assert_eq!(users.get("alice").await?.as_deref(), Some(&b"active"[..]));
//!
//!     pool.transaction(|transaction| {
//!         let users = users.clone();
//!         Box::pin(async move {
//!             let current = transaction.get(&users, "hits").await?;
//!             let next = current
//!                 .as_deref()
//!                 .and_then(|bytes| std::str::from_utf8(bytes).ok())
//!                 .and_then(|text| text.parse::<u64>().ok())
//!                 .unwrap_or(0)
//!                 + 1;
//!             transaction.put(&users, "hits", next.to_string()).await?;
//!             Ok(())
//!         })
//!     })
//!     .await?;
//!     assert_eq!(users.get("hits").await?.as_deref(), Some(&b"1"[..]));
//!
//!     pool.close().await;
//!     std::fs::remove_dir_all(&path).ok();
//!     Ok(())
//! }
//! ```
//!
//! # Feature flags
//!
//! - `json` (default): typed collections use JSON through `serde_json`.
//! - `postcard`: typed collections can use Postcard through `postcard`.
//! - `serde`: enable the typed collection API without selecting a codec.
//!
//! # Documentation
//!
//! - [SDK overview](https://github.com/HardBoss07/storkv/blob/main/docs/storkv-sdk/index.md)
//! - [Rust SDK guide](https://github.com/HardBoss07/storkv/blob/main/docs/storkv-sdk/rust.md)
//! - [C ABI reference](https://github.com/HardBoss07/storkv/blob/main/docs/reference/ffi-c-abi.md)

#![warn(missing_docs)]

mod backend;
mod cache;
mod error;
mod options;
mod pool;
mod scope;
mod transaction;
mod watch;

#[cfg(any(feature = "json", feature = "postcard"))]
mod collection;

#[cfg(any(feature = "json", feature = "postcard"))]
pub use collection::{Collection, TypedWatchEvent, TypedWatchStream};
pub use error::{Result, SdkError};
pub use options::{CacheOptions, RetryPolicy, ScopeDefinition, StorkvPoolOptions};
pub use pool::{PoolConnection, StorkvPool, TxFuture};
pub use scope::Scope;
pub use transaction::Transaction;
pub use watch::{WatchEvent, WatchEventKind, WatchStream};

pub use storkv_core::{CompactionStyle, Durability, PutCondition, PutIfResult, Retention};
pub use storkv_proto::Consistency;

const _: () = {
    assert!(std::mem::size_of::<WatchEventKind>() == 1);
};
