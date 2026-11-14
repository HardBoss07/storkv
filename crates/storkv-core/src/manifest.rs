//! MANIFEST version-edit log: record codecs, CRC-framed edits, in-memory
//! version state, and atomic `CURRENT` activation.

mod codec;
mod manager;
mod record;
mod version_set;

pub use codec::VersionEdit;
pub use manager::ManifestManager;
pub use record::{
    CompactionStyleMode, DurabilityMode, EditRecord, MANIFEST_MAGIC, MANIFEST_VERSION,
    RetentionMode, ScopeMeta, SnapshotMeta, SstFileMeta,
};
pub use version_set::ManifestState;
