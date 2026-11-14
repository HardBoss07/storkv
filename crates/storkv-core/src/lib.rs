#![allow(
    clippy::collapsible_if,
    clippy::new_without_default,
    clippy::map_entry,
    clippy::suspicious_open_options
)]

pub mod compaction;
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
