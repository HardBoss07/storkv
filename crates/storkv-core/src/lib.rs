//! StorKV embedded storage engine.
//!
//! `storkv-core` implements an LSM-tree key/value store with MVCC snapshot
//! isolation, scoped isolation, TTL retention, prefix Bloom filters, merge
//! operators, watch streams, and continuous backup with point-in-time restore.
//! The crate is asynchronous-runtime agnostic: it uses only blocking file I/O
//! and standard synchronisation primitives, so it embeds in any Rust process.
//!
//! # Quick start
//!
//! ```
//! use storkv_core::{Db, DbOptions, ScopeOptions};
//!
//! # fn main() -> storkv_core::Result<()> {
//! let dir = std::env::temp_dir().join("storkv-doc-quick-start");
//! let _ = std::fs::remove_dir_all(&dir);
//! let db = Db::open(&dir, DbOptions::default())?;
//! let scope = db.create_scope("sessions", ScopeOptions::default())?;
//! db.put(&scope, b"hello", b"world")?;
//! assert_eq!(db.get(&scope, b"hello")?.as_deref(), Some(&b"world"[..]));
//! db.delete(&scope, b"hello")?;
//! # drop(db);
//! # let _ = std::fs::remove_dir_all(&dir);
//! # Ok(())
//! # }
//! ```
//!
//! # Module map
//!
//! - [`Db`] and [`ScopeHandle`]: database handle, scope handles, and opening.
//! - [`Transaction`]: optimistic transactions with read-set validation.
//! - [`wal`], [`sstable`], and [`manifest`]: durable on-disk structures.
//! - [`mvcc`], [`memtable`], and [`compaction`]: the in-memory write path.
//! - [`backup`]: automatic backup, WAL shipping, verification, and restore.
//! - [`compat`]: embedded `Tree` and `Batch` adapters.
//! - [`vfs`]: filesystem abstraction and fault injection.
//! - [`watch`] and [`telemetry`]: change streams and runtime counters.

#![warn(missing_docs)]
#![allow(
    clippy::collapsible_if,
    clippy::new_without_default,
    clippy::map_entry,
    clippy::suspicious_open_options
)]

pub mod backup;
pub mod compaction;
pub mod compat;
pub mod error;
pub mod format;
pub mod manifest;
pub mod memtable;
pub mod mvcc;
pub mod options;
pub mod scope;
pub mod sstable;
pub mod telemetry;
pub mod types;
pub mod vfs;
pub mod wal;
pub mod watch;

mod checkpoint;
mod db;
mod db_ops;
mod recovery;
mod transaction;

pub use db::{Db, DbInner, ScopeHandle};
pub use error::{Result, StorKvError};
pub use mvcc::MergeOp;
pub use options::DbOptions;
pub use scope::{CompactionStyle, Retention, ScopeOptions};
pub use transaction::Transaction;
pub use types::{FileNumber, ScopeId, SequenceNumber, TimestampMs};
pub use wal::Durability;
