//! Archive framing, compression, and entry collection.

use std::path::Path;

use crate::error::{Result, StorkvError};

use super::super::config::BackupCompression;
use super::super::manifest::{BackupFileKind, MANIFEST_FILENAME, manifest_path_string};
use super::{ENTRY_END, ENTRY_OTHER, ENTRY_WAL};

/// Compression tag for an uncompressed payload.
const COMPRESSION_NONE: u8 = 0;
/// Compression tag for a Zstandard payload.
const COMPRESSION_ZSTD: u8 = 1;
/// Compression tag for an LZ4 payload.
const COMPRESSION_LZ4: u8 = 2;
/// Zstandard compression level used for backup payloads.
const ZSTD_COMPRESSION_LEVEL: i32 = 3;

pub(super) fn encode_entries(entries: &[(u8, String, Vec<u8>)]) -> Result<Vec<u8>> {
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

pub(super) fn compress_payload(
    payload: Vec<u8>,
    compression: BackupCompression,
) -> Result<(u8, Vec<u8>)> {
    match compression {
        BackupCompression::None => Ok((COMPRESSION_NONE, payload)),
        BackupCompression::Zstd => {
            let compressed = zstd::stream::encode_all(payload.as_slice(), ZSTD_COMPRESSION_LEVEL)
                .map_err(|error| {
                StorkvError::BackupSink {
                    sink: "command-pipe".into(),
                    detail: format!("zstd compression failed: {error}"),
                }
            })?;
            Ok((COMPRESSION_ZSTD, compressed))
        }
        BackupCompression::Lz4 => Ok((
            COMPRESSION_LZ4,
            lz4_flex::compress_prepend_size(&payload),
        )),
    }
}

pub(super) fn collect_entries(
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
            StorkvError::InvalidOperation(format!("backup path not under root: {error}"))
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
            _ => ENTRY_OTHER,
        };
        entries.push((kind, relative_name, bytes));
    }
    Ok(())
}
