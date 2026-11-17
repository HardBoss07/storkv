use std::path::Path;
use std::sync::Arc;

use crate::error::{Result, StorKvError};
use crate::vfs::FileSystem;

use super::record::WalBatchFrame;
use super::writer::{WAL_HEADER_SIZE, WAL_MAGIC};

/// Recovery reader for WAL segments:
/// Handles torn tail auto-truncation and mid-segment corruption errors.
pub struct WalRecovery;

impl WalRecovery {
    /// Read every complete batch from one WAL segment.
    ///
    /// A torn tail on the last segment is truncated and ignored. A checksum
    /// failure in a non-last segment returns [`StorKvError::Corruption`].
    ///
    /// # Errors
    ///
    /// Returns an error when the segment cannot be read or is corrupt.
    pub fn recover_segment<FS: FileSystem>(
        fs: Arc<FS>,
        path: &Path,
        is_last_segment: bool,
    ) -> Result<Vec<WalBatchFrame>> {
        let data = read_segment_bytes(fs.as_ref(), path)?;
        validate_segment_header(&data, is_last_segment)?;
        replay_frames(fs.as_ref(), path, &data, is_last_segment)
    }
}

fn read_segment_bytes<FS: FileSystem>(fs: &FS, path: &Path) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    let mut file = fs.open_file(path)?;
    file.read_to_end(&mut data)?;
    Ok(data)
}

fn validate_segment_header(data: &[u8], is_last_segment: bool) -> Result<()> {
    if data.len() < WAL_HEADER_SIZE {
        return if is_last_segment {
            Ok(())
        } else {
            Err(StorKvError::Corruption(
                "Truncated WAL segment header".into(),
            ))
        };
    }
    if &data[0..8] != WAL_MAGIC {
        return Err(StorKvError::Corruption("Invalid WAL magic".into()));
    }
    Ok(())
}

fn replay_frames<FS: FileSystem>(
    fs: &FS,
    path: &Path,
    data: &[u8],
    is_last_segment: bool,
) -> Result<Vec<WalBatchFrame>> {
    if data.len() < WAL_HEADER_SIZE {
        return Ok(Vec::new());
    }

    let mut frames = Vec::new();
    let mut offset = WAL_HEADER_SIZE;
    while offset < data.len() {
        match WalBatchFrame::decode(&data[offset..]) {
            Ok((frame, consumed)) => {
                frames.push(frame);
                offset += consumed;
            }
            Err(error) => {
                if !is_last_segment {
                    return Err(error);
                }
                truncate_torn_tail(fs, path, offset)?;
                break;
            }
        }
    }
    Ok(frames)
}

fn truncate_torn_tail<FS: FileSystem>(fs: &FS, path: &Path, offset: usize) -> Result<()> {
    let mut app_file = fs.append_file(path)?;
    app_file.set_len(offset as u64)?;
    app_file.sync_all()?;
    Ok(())
}
