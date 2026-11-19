use std::path::Path;

use crate::error::{Result, StorkvError};

use super::io::crc32c_file;
use super::paths::manifest_path_string;
use super::{BackupFileEntry, BackupFileKind, MANIFEST_CRC_SUFFIX, MANIFEST_FILENAME};
use crate::backup::wal_index;

/// Build file entries for every file currently present in a staged generation.
pub fn collect_file_entries(staged_dir: &Path) -> Result<Vec<BackupFileEntry>> {
    let mut entries = Vec::new();
    collect_recursive(staged_dir, staged_dir, &mut entries)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn collect_recursive(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<BackupFileEntry>,
) -> Result<()> {
    let mut children = std::fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    children.sort_by_key(|entry| entry.path());
    for child in children {
        let path = child.path();
        if path.is_dir() {
            collect_recursive(root, &path, entries)?;
            continue;
        }
        let relative_path = path.strip_prefix(root).map_err(|error| {
            StorkvError::InvalidOperation(format!("backup path not under root: {error}"))
        })?;
        let relative_name = manifest_path_string(relative_path);
        if relative_name == MANIFEST_FILENAME
            || relative_name == format!("{MANIFEST_FILENAME}{MANIFEST_CRC_SUFFIX}")
        {
            continue;
        }
        entries.push(build_file_entry(&path, &relative_name)?);
    }
    Ok(())
}

fn build_file_entry(path: &Path, relative_name: &str) -> Result<BackupFileEntry> {
    let kind = BackupFileKind::from_relative_path(Path::new(relative_name));
    let size_bytes = std::fs::metadata(path)?.len();
    let crc32c = crc32c_file(path)?;
    let (min_seq, max_seq) = if kind == BackupFileKind::Wal {
        let range = wal_index::inspect_segment(path)?;
        (Some(range.min_seq), Some(range.max_seq))
    } else {
        (None, None)
    };
    Ok(BackupFileEntry {
        path: relative_name.to_string(),
        kind,
        size_bytes,
        crc32c: format!("0x{crc32c:08x}"),
        min_seq,
        max_seq,
    })
}
