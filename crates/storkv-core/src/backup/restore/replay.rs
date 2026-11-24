//! WAL segment filtering and replay for point-in-time restore.

use std::path::Path;

use crate::backup::manifest::write_synced;
use crate::backup::wal_index::{self, WAL_CREATED_AT_LEN, WAL_CREATED_AT_OFFSET};
use crate::error::{Result, StorkvError};
use crate::wal::{WAL_HEADER_SIZE, WAL_MAGIC, WalBatchFrame};

pub(super) fn replay_segments(
    backup_root: &Path,
    destination_wal: &Path,
    target_seq: u64,
) -> Result<usize> {
    let base_segments = wal_index::list_segments(destination_wal)?;
    let max_base_segment = base_segments
        .iter()
        .map(|segment| segment.file_num)
        .max()
        .unwrap_or(0);

    let mut wal_segments_replayed = 0usize;
    for segment in wal_index::list_segments(&backup_root.join("wal"))? {
        if segment.file_num <= max_base_segment {
            continue;
        }
        if segment.min_seq > target_seq {
            continue;
        }
        if replay_segment_filtered(&segment.path, destination_wal, target_seq)? {
            wal_segments_replayed += 1;
        }
    }
    Ok(wal_segments_replayed)
}

fn replay_segment_filtered(source: &Path, destination_wal: &Path, target_seq: u64) -> Result<bool> {
    let bytes = std::fs::read(source)?;
    if bytes.len() < WAL_HEADER_SIZE || &bytes[0..8] != WAL_MAGIC {
        return Err(StorkvError::BackupCorruption {
            path: source.display().to_string(),
            detail: "invalid WAL header".into(),
        });
    }
    std::fs::create_dir_all(destination_wal)?;

    let mut output =
        wal_header(&bytes[WAL_CREATED_AT_OFFSET..WAL_CREATED_AT_OFFSET + WAL_CREATED_AT_LEN]);
    let mut offset = WAL_HEADER_SIZE;
    let mut kept_any = false;
    while offset < bytes.len() {
        let (frame, consumed) = WalBatchFrame::decode(&bytes[offset..]).map_err(|error| {
            StorkvError::BackupCorruption {
                path: source.display().to_string(),
                detail: error.to_string(),
            }
        })?;
        offset += consumed;
        let kept_ops = frame
            .ops
            .iter()
            .enumerate()
            .take_while(|(index, _)| frame.batch_seq.saturating_add(*index as u64) <= target_seq)
            .count();
        if kept_ops == 0 {
            continue;
        }
        let filtered = WalBatchFrame::new(frame.batch_seq, frame.ops[..kept_ops].to_vec());
        output.extend_from_slice(&filtered.encode(false)?);
        kept_any = true;
        if kept_ops < frame.ops.len() {
            break;
        }
    }

    if !kept_any {
        return Ok(false);
    }
    let file_name = source
        .file_name()
        .ok_or_else(|| StorkvError::InvalidOperation("WAL segment has no file name".into()))?;
    write_synced(&destination_wal.join(file_name), &output)?;
    Ok(true)
}

fn wal_header(created_at_sec: &[u8]) -> Vec<u8> {
    let mut header = Vec::with_capacity(WAL_HEADER_SIZE);
    header.extend_from_slice(WAL_MAGIC);
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&0u16.to_le_bytes());
    if created_at_sec.len() == WAL_CREATED_AT_LEN {
        header.extend_from_slice(created_at_sec);
    } else {
        header.extend_from_slice(&0u32.to_le_bytes());
    }
    header
}
