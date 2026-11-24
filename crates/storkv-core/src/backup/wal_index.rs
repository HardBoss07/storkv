use std::path::{Path, PathBuf};

use crate::error::{Result, StorkvError};
use crate::wal::{WAL_HEADER_SIZE, WAL_MAGIC, WalBatchFrame};

/// Number of hexadecimal digits in a WAL segment file name.
const WAL_SEGMENT_NAME_HEX_LEN: usize = 16;
/// Offset of the creation timestamp field in a WAL segment header.
pub(crate) const WAL_CREATED_AT_OFFSET: usize = 12;
/// Size of the creation timestamp field in a WAL segment header.
pub(crate) const WAL_CREATED_AT_LEN: usize = 4;

/// Sequence span and metadata of one WAL segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalSegmentInfo {
    /// Unix epoch seconds when the segment was created.
    pub created_at_sec: u32,
    /// Segment file number.
    pub file_num: u64,
    /// Oldest batch sequence number in the segment.
    pub min_seq: u64,
    /// Newest batch sequence number in the segment.
    pub max_seq: u64,
    /// Segment size in bytes.
    pub size_bytes: u64,
    /// Filesystem path of the segment.
    pub path: PathBuf,
}

/// Read a sealed or active WAL segment and derive its sequence span.
///
/// This never mutates the segment and does not verify the sealed flag; callers
/// only ship segments strictly below `WalManager::current_file_num()`.
pub fn inspect_segment(path: &Path) -> Result<WalSegmentInfo> {
    let bytes = std::fs::read(path)?;
    validate_header(&bytes, path)?;
    let created_at_sec = u32::from_le_bytes(
        bytes[WAL_CREATED_AT_OFFSET..WAL_CREATED_AT_OFFSET + WAL_CREATED_AT_LEN]
            .try_into()
            .map_err(|_| corruption(path, "truncated WAL creation timestamp"))?,
    );

    let parsed = parse_file_number(path)
        .ok_or_else(|| corruption(path, "WAL file name is not a 16-digit hex segment number"))?;
    let mut min_seq = u64::MAX;
    let mut max_seq = 0u64;
    let mut offset = WAL_HEADER_SIZE;
    while offset < bytes.len() {
        let (frame, consumed) = WalBatchFrame::decode(&bytes[offset..])
            .map_err(|error| corruption(path, &format!("invalid WAL frame: {error}")))?;
        if consumed == 0 {
            return Err(corruption(path, "WAL frame decoded with zero length"));
        }
        let frame_min = frame.batch_seq;
        let frame_max = frame
            .batch_seq
            .saturating_add(frame.ops.len().saturating_sub(1) as u64);
        min_seq = min_seq.min(frame_min);
        max_seq = max_seq.max(frame_max);
        offset += consumed;
    }
    if min_seq == u64::MAX {
        min_seq = 0;
        max_seq = 0;
    }
    Ok(WalSegmentInfo {
        file_num: parsed,
        path: path.to_path_buf(),
        min_seq,
        max_seq,
        created_at_sec,
        size_bytes: bytes.len() as u64,
    })
}

/// Enumerate `*.wal` files in a directory in ascending segment-number order.
pub fn list_segments(wal_dir: &Path) -> Result<Vec<WalSegmentInfo>> {
    if !wal_dir.exists() {
        return Ok(Vec::new());
    }
    let mut segments = Vec::new();
    for entry in std::fs::read_dir(wal_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("wal") {
            continue;
        }
        if parse_file_number(&path).is_none() {
            continue;
        }
        segments.push(inspect_segment(&path)?);
    }
    segments.sort_by_key(|segment| segment.file_num);
    Ok(segments)
}

/// Parse a 16-digit hexadecimal WAL segment file name.
pub fn parse_file_number(path: &Path) -> Option<u64> {
    let stem = path.file_stem()?.to_str()?;
    if stem.len() != WAL_SEGMENT_NAME_HEX_LEN {
        return None;
    }
    u64::from_str_radix(stem, 16).ok()
}

fn validate_header(bytes: &[u8], path: &Path) -> Result<()> {
    if bytes.len() < WAL_HEADER_SIZE {
        return Err(corruption(path, "WAL segment is shorter than its header"));
    }
    if &bytes[0..8] != WAL_MAGIC {
        return Err(corruption(path, "invalid WAL magic"));
    }
    Ok(())
}

fn corruption(path: &Path, detail: &str) -> StorkvError {
    StorkvError::BackupCorruption {
        path: path.display().to_string(),
        detail: detail.to_string(),
    }
}
