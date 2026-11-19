use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::db::DbInner;
use crate::error::{Result, StorkvError};
use crate::vfs::{AnyFs, PosixFs};

use crate::backup::config::BackupConfig;
use crate::backup::manifest::{
    BackupFileEntry, BackupFileKind, BackupKind, BackupManifest, collect_file_entries, crc32c_file,
    new_generation_id, new_wal_generation_id, now_timestamp_ms, staging_directory,
};
use crate::backup::{retention, wal_index};

use super::{BackupReport, BackupStatus, SCHEDULER_TICK};

pub(super) struct BackupController {
    pub(super) config: BackupConfig,
    pub(super) inner: Arc<DbInner<AnyFs<PosixFs>>>,
    pub(super) shutdown: AtomicBool,
    pub(super) wake: (Mutex<bool>, parking_lot::Condvar),
    run_lock: Mutex<()>,
    pub(super) next_full_at: Mutex<Instant>,
    last_shipped_wal: Mutex<Option<u64>>,
    last_error: Mutex<Option<String>>,
}

impl BackupController {
    pub(super) fn new(inner: Arc<DbInner<AnyFs<PosixFs>>>, config: BackupConfig) -> Self {
        Self {
            config,
            inner,
            shutdown: AtomicBool::new(false),
            wake: (Mutex::new(false), parking_lot::Condvar::new()),
            run_lock: Mutex::new(()),
            next_full_at: Mutex::new(Instant::now()),
            last_shipped_wal: Mutex::new(None),
            last_error: Mutex::new(None),
        }
    }

    pub(super) fn should_stop(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    pub(super) fn status(&self) -> BackupStatus {
        let telemetry = &self.inner.telemetry;
        let next_full_backup_in_ms = self
            .next_full_at
            .lock()
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        BackupStatus {
            enabled: !self.should_stop(),
            backup_root: Some(self.config.backup_root.display().to_string()),
            last_backup_timestamp_ms: telemetry.last_backup_timestamp_ms.load(Ordering::Relaxed),
            last_backup_seq: telemetry.last_backup_seq.load(Ordering::Relaxed),
            backups_completed_total: telemetry.backups_completed_total.load(Ordering::Relaxed),
            backups_failed_total: telemetry.backups_failed_total.load(Ordering::Relaxed),
            wal_segments_shipped_total: telemetry
                .wal_segments_shipped_total
                .load(Ordering::Relaxed),
            next_full_backup_in_ms,
            last_error: self.last_error.lock().clone(),
            sink: self.config.sink.kind().as_str().to_string(),
        }
    }

    pub(super) fn run_full_backup(&self) -> Result<BackupReport> {
        let _run_guard = self.run_lock.lock();
        let sequence = self.inner.last_visible_seq.load(Ordering::Acquire);
        let timestamp_ms = now_timestamp_ms();
        let generation_id = new_generation_id(timestamp_ms, sequence);
        let staged_dir = staging_directory(&self.config.backup_root, &generation_id)?;

        let result = self.write_generation(&generation_id, timestamp_ms, sequence, &staged_dir);
        match result {
            Ok(report) => {
                self.inner
                    .telemetry
                    .record_backup_completed(timestamp_ms, sequence);
                *self.last_error.lock() = None;
                Ok(report)
            }
            Err(error) => {
                let _ = std::fs::remove_dir_all(&staged_dir);
                self.inner.telemetry.record_backup_failed();
                *self.last_error.lock() = Some(error.to_string());
                Err(error)
            }
        }
    }

    fn write_generation(
        &self,
        generation_id: &str,
        timestamp_ms: u64,
        sequence: u64,
        staged_dir: &std::path::Path,
    ) -> Result<BackupReport> {
        if staged_dir.exists() {
            std::fs::remove_dir_all(staged_dir)?;
        }
        self.inner.checkpoint(staged_dir)?;

        let files = collect_file_entries(staged_dir)?;
        let total_bytes = files.iter().map(|entry| entry.size_bytes).sum();
        let min_seq = files
            .iter()
            .filter_map(|entry| entry.min_seq)
            .min()
            .unwrap_or(0)
            .min(sequence);
        let mut manifest = BackupManifest::new(
            generation_id.to_string(),
            BackupKind::Full,
            timestamp_ms,
            min_seq,
            sequence,
            self.config.compression.as_str(),
        );
        manifest.total_bytes = total_bytes;
        manifest.files = files;
        manifest.write_to_dir(staged_dir)?;

        self.config.sink.publish_generation(&manifest, staged_dir)?;

        let plan = retention::build_plan(&self.config.backup_root, &self.config.retention)?;
        let prune = self.config.sink.prune(&plan)?;
        Ok(BackupReport {
            generation_id: generation_id.to_string(),
            sequence,
            files: manifest.files.len(),
            bytes: total_bytes,
            prune,
        })
    }

    fn ship_sealed_wal(&self) -> Result<usize> {
        let current_file_num = self.inner.wal_mgr.current_file_num();
        let mut last_shipped = self.last_shipped_wal.lock();
        let Some(last_number) = *last_shipped else {
            *last_shipped = Some(current_file_num);
            return Ok(0);
        };
        let mut shipped = 0usize;
        for segment in wal_index::list_segments(&self.inner.wal_dir)? {
            if segment.file_num >= current_file_num || segment.file_num <= last_number {
                continue;
            }
            let manifest = wal_shipment_manifest(&segment)?;
            match self.config.sink.ship_wal_segment(&manifest, &segment.path) {
                Ok(()) => {
                    self.inner.telemetry.record_wal_segment_shipped();
                    *last_shipped = Some(segment.file_num);
                    shipped += 1;
                    *self.last_error.lock() = None;
                }
                Err(error) => {
                    *self.last_error.lock() = Some(error.to_string());
                    return Err(error);
                }
            }
        }
        Ok(shipped)
    }
}

fn wal_shipment_manifest(segment: &wal_index::WalSegmentInfo) -> Result<BackupManifest> {
    let file_name = segment
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| StorkvError::InvalidOperation("WAL segment has no file name".into()))?;
    let checksum = crc32c_file(&segment.path)?;
    let entry = BackupFileEntry {
        path: format!("wal/{file_name}"),
        kind: BackupFileKind::Wal,
        size_bytes: segment.size_bytes,
        crc32c: format!("0x{checksum:08x}"),
        min_seq: Some(segment.min_seq),
        max_seq: Some(segment.max_seq),
    };
    let mut manifest = BackupManifest::new(
        new_wal_generation_id(segment.file_num),
        BackupKind::Wal,
        now_timestamp_ms(),
        segment.min_seq,
        segment.max_seq,
        "none",
    );
    manifest.total_bytes = segment.size_bytes;
    manifest.files.push(entry);
    Ok(manifest)
}

pub(super) fn run_worker(controller: Arc<BackupController>) {
    while !controller.should_stop() {
        let now = Instant::now();
        let due_at = *controller.next_full_at.lock();
        if now >= due_at {
            let _ = controller.run_full_backup();
            *controller.next_full_at.lock() =
                Instant::now() + controller.config.full_backup_interval;
        }
        if controller.config.ship_sealed_wal {
            let _ = controller.ship_sealed_wal();
        }

        let (wake_lock, wake_condvar) = &controller.wake;
        let mut wake_requested = wake_lock.lock();
        if controller.should_stop() {
            break;
        }
        if *wake_requested {
            *wake_requested = false;
            *controller.next_full_at.lock() = Instant::now();
            continue;
        }
        let next_due = *controller.next_full_at.lock();
        let timeout = next_due
            .saturating_duration_since(Instant::now())
            .min(SCHEDULER_TICK)
            .max(Duration::from_millis(10));
        wake_condvar.wait_for(&mut wake_requested, timeout);
    }
}
