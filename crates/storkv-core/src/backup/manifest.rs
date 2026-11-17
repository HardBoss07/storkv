use serde::{Deserialize, Serialize};

use crate::error::{Result, StorKvError};

mod files;
mod io;
mod paths;

pub use files::collect_file_entries;
pub use io::{crc32c_file, parse_crc32c, write_synced};
pub use paths::{
    generation_directory, manifest_path_string, manifest_relative_path, new_generation_id,
    new_wal_generation_id, now_timestamp_ms, staging_directory,
};

/// File name of the backup manifest inside a generation directory.
pub const MANIFEST_FILENAME: &str = "BACKUP_MANIFEST.json";
/// Suffix of the sidecar file that holds the manifest checksum.
pub const MANIFEST_CRC_SUFFIX: &str = ".crc32c";
/// Format marker stored in every backup manifest.
pub const BACKUP_FORMAT: &str = "STORKBACKUP";
/// Current backup manifest version.
pub const BACKUP_FORMAT_VERSION: u16 = 1;
pub(crate) const CRC_READ_CHUNK: usize = 64 * 1024;

/// Type of an archived backup generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupKind {
    /// A full database image.
    Full,
    /// A stream of sealed WAL segments.
    Wal,
}

/// Kind of file listed in a backup manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupFileKind {
    /// An SSTable file.
    Sst,
    /// A WAL segment file.
    Wal,
    /// A manifest file.
    Manifest,
    /// The `CURRENT` pointer file.
    Current,
}

impl BackupFileKind {
    /// Classify a file by its name. Unknown names count as manifest files.
    pub fn from_relative_path(relative_path: &std::path::Path) -> Self {
        let name = relative_path
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .unwrap_or_default();
        if name.ends_with(".sst") {
            BackupFileKind::Sst
        } else if name.ends_with(".wal") {
            BackupFileKind::Wal
        } else if name == "CURRENT" {
            BackupFileKind::Current
        } else {
            BackupFileKind::Manifest
        }
    }
}

/// One file entry in a backup manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupFileEntry {
    /// Path relative to the generation directory.
    pub path: String,
    /// File kind.
    pub kind: BackupFileKind,
    /// File size in bytes.
    pub size_bytes: u64,
    /// CRC32C checksum as a hexadecimal string.
    pub crc32c: String,
    /// Oldest sequence number in the file, when the file carries data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_seq: Option<u64>,
    /// Newest sequence number in the file, when the file carries data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_seq: Option<u64>,
}

/// Manifest that describes one backup generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupManifest {
    /// Format marker. Always [`BACKUP_FORMAT`].
    pub format: String,
    /// Manifest format version.
    pub format_version: u16,
    /// Unique identifier of the generation.
    pub generation_id: String,
    /// Generation kind.
    pub kind: BackupKind,
    /// Unix epoch milliseconds when the generation was created.
    pub created_at_ms: u64,
    /// Oldest sequence number in the generation.
    pub min_seq: u64,
    /// Newest sequence number in the generation.
    pub max_seq: u64,
    /// Full generation that a WAL generation extends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_generation: Option<String>,
    /// Compression codec name.
    pub compression: String,
    /// Total bytes of all listed files.
    pub total_bytes: u64,
    /// Files in the generation.
    pub files: Vec<BackupFileEntry>,
}

impl BackupManifest {
    /// Build an empty manifest for a new generation.
    pub fn new(
        generation_id: String,
        kind: BackupKind,
        created_at_ms: u64,
        min_seq: u64,
        max_seq: u64,
        compression: &str,
    ) -> Self {
        Self {
            format: BACKUP_FORMAT.to_string(),
            format_version: BACKUP_FORMAT_VERSION,
            generation_id,
            kind,
            created_at_ms,
            min_seq,
            max_seq,
            base_generation: None,
            compression: compression.to_string(),
            total_bytes: 0,
            files: Vec::new(),
        }
    }

    /// Check the format marker, version, and sequence range.
    ///
    /// # Errors
    ///
    /// Returns [`StorKvError::BackupCorruption`] when a field is invalid.
    pub fn validate(&self) -> Result<()> {
        if self.format != BACKUP_FORMAT {
            return Err(StorKvError::BackupCorruption {
                path: self.generation_id.clone(),
                detail: format!("unexpected backup format `{}`", self.format),
            });
        }
        if self.format_version != BACKUP_FORMAT_VERSION {
            return Err(StorKvError::BackupCorruption {
                path: self.generation_id.clone(),
                detail: format!(
                    "unsupported backup format version {} (expected {})",
                    self.format_version, BACKUP_FORMAT_VERSION
                ),
            });
        }
        if self.max_seq < self.min_seq {
            return Err(StorKvError::BackupCorruption {
                path: self.generation_id.clone(),
                detail: "max_seq is smaller than min_seq".into(),
            });
        }
        Ok(())
    }
}
