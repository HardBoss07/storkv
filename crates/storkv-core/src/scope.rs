//! Scope metadata, lifecycle, and MVCC-aware key lookup across memtables and
//! SSTable levels.

mod meta;
mod options;
mod read;
mod state;

pub use meta::KeyMeta;
pub use options::{
    CompactionStyle, DEFAULT_MAX_WAL_PIN_AGE, DEFAULT_MAX_WAL_PIN_BYTES, Retention, ScopeOptions,
};
pub use state::Scope;
