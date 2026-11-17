use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use crate::error::{Result, StorKvError};

use super::config::BackupCompression;
use super::manifest::{BackupFileKind, BackupManifest, MANIFEST_FILENAME, manifest_path_string};
use super::sink::{BackupSink, PruneReport, RetentionPlan, SinkKind, SinkStatus};

/// Magic bytes of a command-pipe backup archive.
pub const ARCHIVE_MAGIC: &[u8; 8] = b"STORKBAK";
/// Version of the command-pipe archive format.
pub const ARCHIVE_VERSION: u16 = 1;
const ENTRY_END: u8 = 0;
const ENTRY_WAL: u8 = 2;
const ENTRY_MANIFEST: u8 = 3;

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
    /// Returns [`StorKvError::InvalidOperation`] when the program is empty.
    pub fn new(config: CommandPipeSinkConfig) -> Result<Self> {
        if config.program.trim().is_empty() {
            return Err(StorKvError::InvalidOperation(
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

    fn record_error(&self, error: &StorKvError) {
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
            collect_entries(staged_dir, staged_dir, &mut entries)?;
            let manifest_bytes = serde_json::to_vec_pretty(manifest)
                .map_err(|error| StorKvError::Serialization(error.to_string()))?;
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
                .ok_or_else(|| StorKvError::InvalidOperation("WAL segment has no name".into()))?;
            let bytes = std::fs::read(source)?;
            let manifest_bytes = serde_json::to_vec_pretty(manifest)
                .map_err(|error| StorKvError::Serialization(error.to_string()))?;
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
        let payload = encode_entries(entries)?;
        let (compression_tag, payload) = compress_payload(payload, self.config.compression)?;
        let mut archive = Vec::with_capacity(payload.len() + 11);
        archive.extend_from_slice(ARCHIVE_MAGIC);
        archive.extend_from_slice(&ARCHIVE_VERSION.to_le_bytes());
        archive.push(compression_tag);
        archive.extend_from_slice(&payload);
        self.run_child(&archive)
    }

    fn run_child(&self, archive: &[u8]) -> Result<()> {
        let mut child = Command::new(&self.config.program)
            .args(&self.config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| StorKvError::BackupSink {
                sink: "command-pipe".into(),
                detail: format!("failed to spawn `{}`: {error}", self.config.program),
            })?;

        let write_result = child
            .stdin
            .take()
            .ok_or_else(|| StorKvError::BackupSink {
                sink: "command-pipe".into(),
                detail: "child stdin is unavailable".into(),
            })
            .and_then(|mut stdin| {
                stdin
                    .write_all(archive)
                    .map_err(|error| StorKvError::BackupSink {
                        sink: "command-pipe".into(),
                        detail: format!("write to child stdin failed: {error}"),
                    })
            });

        let output = child
            .wait_with_output()
            .map_err(|error| StorKvError::BackupSink {
                sink: "command-pipe".into(),
                detail: format!("failed to wait for child: {error}"),
            })?;
        write_result?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(StorKvError::BackupSink {
                sink: "command-pipe".into(),
                detail: format!("command exited with {}: {}", output.status, stderr.trim()),
            });
        }
        Ok(())
    }
}

fn encode_entries(entries: &[(u8, String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut encoded = Vec::new();
    for (kind, path, bytes) in entries {
        let path_bytes = path.as_bytes();
        encoded.push(*kind);
        encoded.extend_from_slice(&(path_bytes.len() as u32).to_be_bytes());
        encoded.extend_from_slice(path_bytes);
        encoded.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        encoded.extend_from_slice(&crate::format::compute_crc32c(bytes).to_le_bytes());
        encoded.extend_from_slice(bytes);
    }
    encoded.push(ENTRY_END);
    Ok(encoded)
}

fn compress_payload(payload: Vec<u8>, compression: BackupCompression) -> Result<(u8, Vec<u8>)> {
    match compression {
        BackupCompression::None => Ok((0, payload)),
        BackupCompression::Zstd => {
            let compressed = zstd::stream::encode_all(payload.as_slice(), 3).map_err(|error| {
                StorKvError::BackupSink {
                    sink: "command-pipe".into(),
                    detail: format!("zstd compression failed: {error}"),
                }
            })?;
            Ok((1, compressed))
        }
        BackupCompression::Lz4 => Ok((2, lz4_flex::compress_prepend_size(&payload))),
    }
}

fn collect_entries(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<(u8, String, Vec<u8>)>,
) -> Result<()> {
    let mut children = std::fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    children.sort_by_key(|entry| entry.path());
    for child in children {
        let path = child.path();
        if path.is_dir() {
            collect_entries(root, &path, entries)?;
            continue;
        }
        let relative_path = path.strip_prefix(root).map_err(|error| {
            StorKvError::InvalidOperation(format!("backup path not under root: {error}"))
        })?;
        let relative_name = manifest_path_string(relative_path);
        if relative_name.ends_with(".crc32c")
            || relative_name == MANIFEST_FILENAME
            || relative_name.ends_with(".manifest.json")
        {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        let kind = match BackupFileKind::from_relative_path(relative_path) {
            BackupFileKind::Wal => ENTRY_WAL,
            _ => 1,
        };
        entries.push((kind, relative_name, bytes));
    }
    Ok(())
}
