use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{Result, StorkvError};

use super::manifest::{BackupManifest, MANIFEST_CRC_SUFFIX, parse_crc32c, write_synced};
use super::sink::{BackupSink, PruneReport, RetentionPlan, SinkKind, SinkStatus};

/// Backup sink that writes generations under a local directory.
pub struct LocalFilesystemSink {
    root: PathBuf,
    status: Mutex<SinkStatus>,
}

impl LocalFilesystemSink {
    /// Create a sink rooted at the given directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            status: Mutex::new(SinkStatus {
                kind: SinkKind::LocalFilesystem,
                last_error: None,
            }),
        }
    }

    /// Return the sink root directory.
    pub fn root(&self) -> &Path {
        &self.root
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

    fn wal_directory(&self) -> PathBuf {
        self.root.join("wal")
    }
}

impl BackupSink for LocalFilesystemSink {
    fn kind(&self) -> SinkKind {
        SinkKind::LocalFilesystem
    }

    fn publish_generation(&self, manifest: &BackupManifest, staged_dir: &Path) -> Result<()> {
        match publish_generation_inner(&self.root, manifest, staged_dir) {
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
        match ship_wal_segment_inner(&self.root, manifest, source) {
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

    fn prune(&self, plan: &RetentionPlan) -> Result<PruneReport> {
        let mut report = PruneReport::default();
        for generation_id in &plan.generations {
            let directory = self.root.join("generations").join(generation_id);
            if !directory.exists() {
                continue;
            }
            report.bytes_freed += directory_size(&directory).unwrap_or_default();
            std::fs::remove_dir_all(&directory)?;
            report.generations_removed += 1;
        }

        let wal_directory = self.wal_directory();
        for file_number in &plan.wal_segments {
            let file_name = format!("{file_number:016x}.wal");
            let segment_path = wal_directory.join(&file_name);
            if !segment_path.exists() {
                continue;
            }
            report.bytes_freed += std::fs::metadata(&segment_path)
                .map(|metadata| metadata.len())
                .unwrap_or_default();
            std::fs::remove_file(&segment_path)?;
            remove_if_exists(&wal_directory.join(format!("{file_name}{MANIFEST_CRC_SUFFIX}")))?;
            remove_if_exists(&wal_directory.join(format!("{file_name}.manifest.json")))?;
            report.wal_segments_removed += 1;
        }

        if wal_directory.exists() {
            sync_directory(&wal_directory);
        }
        let generations_dir = self.root.join("generations");
        if generations_dir.exists() {
            sync_directory(&generations_dir);
        }
        Ok(report)
    }

    fn status(&self) -> SinkStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default()
    }
}

fn publish_generation_inner(
    root: &Path,
    manifest: &BackupManifest,
    staged_dir: &Path,
) -> Result<()> {
    let generations_dir = root.join("generations");
    std::fs::create_dir_all(&generations_dir)?;
    let destination = generations_dir.join(&manifest.generation_id);
    if destination.exists() {
        return Err(StorkvError::InvalidOperation(format!(
            "backup generation already exists: {}",
            destination.display()
        )));
    }
    std::fs::rename(staged_dir, &destination)?;
    sync_directory(&destination);
    sync_directory(&generations_dir);
    Ok(())
}

fn ship_wal_segment_inner(root: &Path, manifest: &BackupManifest, source: &Path) -> Result<()> {
    let wal_directory = root.join("wal");
    std::fs::create_dir_all(&wal_directory)?;
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| StorkvError::InvalidOperation("WAL segment has no file name".into()))?;
    let destination = wal_directory.join(file_name);
    if destination.exists() {
        return Ok(());
    }

    let temporary = wal_directory.join(format!(".{file_name}.tmp"));
    std::fs::copy(source, &temporary)?;
    let temporary_file = std::fs::File::open(&temporary)?;
    temporary_file.sync_all()?;
    std::fs::rename(&temporary, &destination)?;

    let checksum_entry = manifest
        .files
        .iter()
        .find(|entry| entry.path.ends_with(file_name))
        .ok_or_else(|| {
            StorkvError::InvalidOperation("WAL manifest is missing its file entry".into())
        })?;
    let checksum = parse_crc32c(&checksum_entry.crc32c)?;
    write_synced(
        &wal_directory.join(format!("{file_name}{MANIFEST_CRC_SUFFIX}")),
        format!("0x{checksum:08x}\n").as_bytes(),
    )?;
    let manifest_bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| StorkvError::Serialization(error.to_string()))?;
    let manifest_checksum = crate::format::compute_crc32c(&manifest_bytes);
    let manifest_name = format!("{file_name}.manifest.json");
    write_synced(&wal_directory.join(&manifest_name), &manifest_bytes)?;
    write_synced(
        &wal_directory.join(format!("{manifest_name}{MANIFEST_CRC_SUFFIX}")),
        format!("0x{manifest_checksum:08x}\n").as_bytes(),
    )?;
    sync_directory(&wal_directory);
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn directory_size(directory: &Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            total = total.saturating_add(directory_size(&path)?);
        } else {
            total = total.saturating_add(std::fs::metadata(&path)?.len());
        }
    }
    Ok(total)
}

fn sync_directory(directory: &Path) {
    #[cfg(unix)]
    if let Ok(file) = std::fs::File::open(directory) {
        let _ = file.sync_all();
    }
    #[cfg(not(unix))]
    let _ = directory;
}
