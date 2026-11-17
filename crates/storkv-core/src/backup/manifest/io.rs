use std::io::{Read, Write};
use std::path::Path;

use crate::error::{Result, StorKvError};

use super::{BackupManifest, CRC_READ_CHUNK, MANIFEST_CRC_SUFFIX, MANIFEST_FILENAME};

impl BackupManifest {
    /// Write the manifest and its CRC32C sidecar file into `directory`.
    ///
    /// # Errors
    ///
    /// Returns an error when serialization or the file write fails.
    pub fn write_to_dir(&self, directory: &Path) -> Result<()> {
        std::fs::create_dir_all(directory)?;
        let manifest_bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| StorKvError::Serialization(error.to_string()))?;
        let checksum = crate::format::compute_crc32c(&manifest_bytes);

        let manifest_path = directory.join(MANIFEST_FILENAME);
        write_synced(&manifest_path, &manifest_bytes)?;
        let checksum_path = directory.join(format!("{MANIFEST_FILENAME}{MANIFEST_CRC_SUFFIX}"));
        write_synced(&checksum_path, crc32c_line(checksum).as_bytes())?;
        Ok(())
    }

    /// Read a manifest and verify its CRC32C checksum.
    ///
    /// # Errors
    ///
    /// Returns [`StorKvError::BackupCorruption`] on a read failure, a checksum
    /// mismatch, invalid JSON, or an invalid manifest.
    pub fn read_from_dir(directory: &Path) -> Result<Self> {
        let manifest_path = directory.join(MANIFEST_FILENAME);
        let checksum_path = directory.join(format!("{MANIFEST_FILENAME}{MANIFEST_CRC_SUFFIX}"));
        let manifest_bytes =
            std::fs::read(&manifest_path).map_err(|error| StorKvError::BackupCorruption {
                path: manifest_path.display().to_string(),
                detail: error.to_string(),
            })?;
        let expected_checksum = read_checksum(&checksum_path)?;
        let actual_checksum = crate::format::compute_crc32c(&manifest_bytes);
        if expected_checksum != actual_checksum {
            return Err(StorKvError::BackupCorruption {
                path: manifest_path.display().to_string(),
                detail: format!(
                    "manifest CRC32C mismatch: expected 0x{expected_checksum:08x}, got 0x{actual_checksum:08x}"
                ),
            });
        }

        let manifest: BackupManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|error| {
                StorKvError::BackupCorruption {
                    path: manifest_path.display().to_string(),
                    detail: error.to_string(),
                }
            })?;
        manifest.validate()?;
        Ok(manifest)
    }
}

pub fn crc32c_file(path: &Path) -> Result<u32> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; CRC_READ_CHUNK];
    let mut checksum = 0u32;
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        checksum = crc32c::crc32c_append(checksum, &buffer[..bytes_read]);
    }
    Ok(checksum)
}

pub fn parse_crc32c(value: &str) -> Result<u32> {
    let trimmed = value.trim();
    let digits = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    u32::from_str_radix(digits, 16).map_err(|error| StorKvError::BackupCorruption {
        path: MANIFEST_FILENAME.into(),
        detail: format!("invalid CRC32C `{value}`: {error}"),
    })
}

pub fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn read_checksum(path: &Path) -> Result<u32> {
    let contents =
        std::fs::read_to_string(path).map_err(|error| StorKvError::BackupCorruption {
            path: path.display().to_string(),
            detail: error.to_string(),
        })?;
    parse_crc32c(&contents)
}

fn crc32c_line(checksum: u32) -> String {
    format!("0x{checksum:08x}\n")
}
