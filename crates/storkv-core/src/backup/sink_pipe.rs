use std::path::Path;
use std::sync::Mutex;

use crate::error::{Result, StorkvError};

use super::config::BackupCompression;
use super::manifest::{BackupManifest, MANIFEST_FILENAME};
use super::sink::{BackupSink, PruneReport, RetentionPlan, SinkKind, SinkStatus};

mod archive;
mod process;

/// Magic bytes of a command-pipe backup archive.
pub const ARCHIVE_MAGIC: &[u8; 8] = b"STORKBAK";
/// Version of the command-pipe archive format.
pub const ARCHIVE_VERSION: u16 = 1;
const ENTRY_END: u8 = 0;
const ENTRY_OTHER: u8 = 1;
const ENTRY_WAL: u8 = 2;
const ENTRY_MANIFEST: u8 = 3;
const ARCHIVE_VERSION_SIZE: usize = std::mem::size_of::<u16>();
const COMPRESSION_TAG_SIZE: usize = std::mem::size_of::<u8>();
const ARCHIVE_HEADER_LEN: usize = ARCHIVE_MAGIC.len() + ARCHIVE_VERSION_SIZE + COMPRESSION_TAG_SIZE;

/// Configuration of a [`CommandPipeSink`].
#[derive(Debug, Clone)]
pub struct CommandPipeSinkConfig {
    /// Program to start.
    pub program: String,
    /// Program arguments.
    pub args: Vec<String>,
    /// Compression codec for the archive payload.
    pub compression: BackupCompression,
}

impl CommandPipeSinkConfig {
    /// Create a configuration for one program.
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            compression: BackupCompression::None,
        }
    }

    /// Set the program arguments.
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// Set the archive compression codec.
    pub fn with_compression(mut self, compression: BackupCompression) -> Self {
        self.compression = compression;
        self
    }
}

/// Backup sink that sends an archive to a child process through stdin.
pub struct CommandPipeSink {
    config: CommandPipeSinkConfig,
    status: Mutex<SinkStatus>,
}

impl CommandPipeSink {
    /// Validate the command and create the sink.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::InvalidOperation`] when the program is empty.
    pub fn new(config: CommandPipeSinkConfig) -> Result<Self> {
        if config.program.trim().is_empty() {
            return Err(StorkvError::InvalidOperation(
                "backup pipe command must not be empty".into(),
            ));
        }
        Ok(Self {
            config,
            status: Mutex::new(SinkStatus {
                kind: SinkKind::CommandPipe,
                last_error: None,
            }),
        })
    }

    fn record_error(&self, error: &StorkvError) {
        if let Ok(mut status) = self.status.lock() {
            status.last_error = Some(error.to_string());
        }
    }

    fn clear_error(&self) {
        if let Ok(mut status) = self.status.lock() {
            status.last_error = None;
        }
    }
}

impl BackupSink for CommandPipeSink {
    fn kind(&self) -> SinkKind {
        SinkKind::CommandPipe
    }

    fn publish_generation(&self, manifest: &BackupManifest, staged_dir: &Path) -> Result<()> {
        let result = (|| {
            let mut entries = Vec::new();
            archive::collect_entries(staged_dir, staged_dir, &mut entries)?;
            let manifest_bytes = serde_json::to_vec_pretty(manifest)
                .map_err(|error| StorkvError::Serialization(error.to_string()))?;
            entries.push((
                ENTRY_MANIFEST,
                MANIFEST_FILENAME.to_string(),
                manifest_bytes,
            ));
            entries.sort_by(|left, right| left.1.cmp(&right.1));
            self.write_archive(&entries)
        })();
        match result {
            Ok(()) => {
                self.clear_error();
                Ok(())
            }
            Err(error) => {
                self.record_error(&error);
                Err(error)
            }
        }
    }

    fn ship_wal_segment(&self, manifest: &BackupManifest, source: &Path) -> Result<()> {
        let result = (|| {
            let file_name = source
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| StorkvError::InvalidOperation("WAL segment has no name".into()))?;
            let bytes = std::fs::read(source)?;
            let manifest_bytes = serde_json::to_vec_pretty(manifest)
                .map_err(|error| StorkvError::Serialization(error.to_string()))?;
            let entries = vec![
                (
                    ENTRY_MANIFEST,
                    format!("wal/{file_name}.manifest.json"),
                    manifest_bytes,
                ),
                (ENTRY_WAL, format!("wal/{file_name}"), bytes),
            ];
            self.write_archive(&entries)
        })();
        match result {
            Ok(()) => {
                self.clear_error();
                Ok(())
            }
            Err(error) => {
                self.record_error(&error);
                Err(error)
            }
        }
    }

    fn prune(&self, _plan: &RetentionPlan) -> Result<PruneReport> {
        Ok(PruneReport::default())
    }

    fn status(&self) -> SinkStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default()
    }
}

impl CommandPipeSink {
    fn write_archive(&self, entries: &[(u8, String, Vec<u8>)]) -> Result<()> {
        let payload = archive::encode_entries(entries)?;
        let (compression_tag, payload) =
            archive::compress_payload(payload, self.config.compression)?;
        let mut archive_bytes = Vec::with_capacity(payload.len() + ARCHIVE_HEADER_LEN);
        archive_bytes.extend_from_slice(ARCHIVE_MAGIC);
        archive_bytes.extend_from_slice(&ARCHIVE_VERSION.to_le_bytes());
        archive_bytes.push(compression_tag);
        archive_bytes.extend_from_slice(&payload);
        process::run_child(self, &archive_bytes)
    }
}
