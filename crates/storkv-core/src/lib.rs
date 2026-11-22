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

/// Continuous backup, WAL shipping, verification, and restore.
pub mod backup;
/// Background compaction scheduling and statistics.
pub mod compaction;
/// Embedded `Tree` and `Batch` adapters.
pub mod compat;
/// Error and result types.
pub mod error;
/// Internal key encoding and decoding.
pub mod format;
/// Manifest log and version metadata.
pub mod manifest;
/// In-memory write buffer.
pub mod memtable;
/// Multi-version concurrency control and merge operators.
pub mod mvcc;
/// Database configuration options.
pub mod options;
/// Scope definitions and retention policies.
pub mod scope;
/// Sorted string table readers, writers, and metadata.
pub mod sstable;
/// Runtime counters and telemetry.
pub mod telemetry;
/// Strongly-typed identifiers.
pub mod types;
/// Filesystem abstraction and fault injection.
pub mod vfs;
/// Write-ahead log records, writer, and recovery.
pub mod wal;
/// Change streams and watch registration.
pub mod watch;

mod checkpoint;
mod db;
mod db_ops;
mod recovery;
mod replication;
mod snapshot;
mod transaction;

pub use db::{Db, DbInner, MAX_SCOPES, ScopeHandle};
pub use error::{Result, StorkvError};
pub use mvcc::{MergeOp, PutCondition, PutIfResult};
pub use options::DbOptions;
pub use replication::ReplicationRegistry;
pub use scope::{CompactionStyle, Retention, ScopeOptions, StorageMode};
pub use transaction::Transaction;
pub use types::{FileNumber, ScopeId, SequenceNumber, TimestampMs};
pub use wal::Durability;

/// One row of a replication snapshot: `(scope_id, key, value, expires_at_ms)`.
pub type ReplicationSnapshotEntry = (u32, bytes::Bytes, bytes::Bytes, u64);

/// A replication snapshot: `(snapshot_seq, live_scope_ids, entries)`.
pub type ReplicationSnapshot = (u64, Vec<u32>, Vec<ReplicationSnapshotEntry>);
