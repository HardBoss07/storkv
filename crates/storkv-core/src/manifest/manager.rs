//! Manifest writer and version state manager.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use bytes::BufMut;
use bytes::BytesMut;

use crate::error::{Result, StorkvError};
use crate::vfs::{File, FileSystem};

use super::codec::VersionEdit;
use super::record::{MANIFEST_MAGIC, MANIFEST_VERSION};
use super::version_set::ManifestState;

const MANIFEST_HEADER_SIZE: usize = 16;
const CURRENT_FILENAME: &str = "CURRENT";
const CURRENT_TEMP_FILENAME: &str = "CURRENT.tmp";
const MANIFEST_PREFIX: &str = "MANIFEST-";

/// Owns the active manifest file and its in-memory state.
pub struct ManifestManager<FS: FileSystem> {
    /// Filesystem backend.
    pub fs: Arc<FS>,
    /// Database root directory.
    pub db_root: PathBuf,
    /// File number of the active manifest.
    pub active_manifest_num: AtomicU64,
    state: parking_lot::RwLock<ManifestState>,
    writer: parking_lot::Mutex<Box<dyn File>>,
}

impl<FS: FileSystem> ManifestManager<FS> {
    /// Open the current manifest or create a new one.
    ///
    /// # Errors
    ///
    /// Returns `StorkvError::Corruption` on an invalid `CURRENT` file or
    /// manifest header, or an I/O error.
    pub fn open(fs: Arc<FS>, db_root: &Path) -> Result<Self> {
        let current_path = db_root.join(CURRENT_FILENAME);
        let (state, active_num, writer) = if fs.exists(&current_path) {
            open_existing_manifest(fs.as_ref(), db_root, &current_path)?
        } else {
            create_new_manifest(fs.as_ref(), db_root, &current_path)?
        };

        Ok(Self {
            fs,
            db_root: db_root.to_path_buf(),
            active_manifest_num: AtomicU64::new(active_num),
            state: parking_lot::RwLock::new(state),
            writer: parking_lot::Mutex::new(writer),
        })
    }

    /// Append one version edit, sync it, and apply it to the in-memory state.
    ///
    /// # Errors
    ///
    /// Returns an error when the edit cannot be written or synced.
    pub fn log_edit(&self, edit: VersionEdit) -> Result<()> {
        let encoded_edit = edit.encode();
        let mut writer = self.writer.lock();
        writer.write_all(&encoded_edit)?;
        writer.sync_all()?;
        let mut state = self.state.write();
        state.apply(&edit);
        Ok(())
    }

    /// Borrow the current in-memory manifest state.
    pub fn state(&self) -> parking_lot::RwLockReadGuard<'_, ManifestState> {
        self.state.read()
    }
}

fn open_existing_manifest<FS: FileSystem>(
    fs: &FS,
    db_root: &Path,
    current_path: &Path,
) -> Result<(ManifestState, u64, Box<dyn File>)> {
    let manifest_filename = read_current_filename(fs, current_path)?;
    let manifest_number = parse_manifest_number(&manifest_filename)?;
    let manifest_path = db_root.join(&manifest_filename);
    let manifest_bytes = read_manifest_bytes(fs, &manifest_path)?;
    validate_manifest_header(&manifest_bytes)?;
    let state = replay_manifest(&manifest_bytes[MANIFEST_HEADER_SIZE..])?;
    let writer = fs.append_file(&manifest_path)?;
    Ok((state, manifest_number, writer))
}

fn create_new_manifest<FS: FileSystem>(
    fs: &FS,
    db_root: &Path,
    current_path: &Path,
) -> Result<(ManifestState, u64, Box<dyn File>)> {
    let manifest_number = 1;
    let manifest_filename = format!("{}{:016x}", MANIFEST_PREFIX, manifest_number);
    let manifest_path = db_root.join(&manifest_filename);
    let mut manifest_file = fs.create_file(&manifest_path)?;
    manifest_file.write_all(&manifest_header())?;
    manifest_file.sync_all()?;

    activate_current(fs, db_root, current_path, &manifest_filename)?;

    let writer = fs.append_file(&manifest_path)?;
    Ok((ManifestState::default(), manifest_number, writer))
}

fn read_current_filename<FS: FileSystem>(fs: &FS, current_path: &Path) -> Result<String> {
    let mut current_file = fs.open_file(current_path)?;
    let mut current_bytes = Vec::new();
    current_file.read_to_end(&mut current_bytes)?;
    let contents = String::from_utf8(current_bytes)
        .map_err(|error| StorkvError::Corruption(error.to_string()))?;
    let manifest_filename = contents.trim().to_string();
    if !manifest_filename.starts_with(MANIFEST_PREFIX) {
        return Err(StorkvError::Corruption(
            "Invalid CURRENT file content".into(),
        ));
    }
    Ok(manifest_filename)
}

fn parse_manifest_number(manifest_filename: &str) -> Result<u64> {
    let number_text = &manifest_filename[MANIFEST_PREFIX.len()..];
    u64::from_str_radix(number_text, 16)
        .map_err(|_| StorkvError::Corruption("Invalid manifest file number in CURRENT".into()))
}

fn read_manifest_bytes<FS: FileSystem>(fs: &FS, manifest_path: &Path) -> Result<Vec<u8>> {
    let mut manifest_file = fs.open_file(manifest_path)?;
    let mut manifest_bytes = Vec::new();
    manifest_file.read_to_end(&mut manifest_bytes)?;
    Ok(manifest_bytes)
}

fn validate_manifest_header(manifest_bytes: &[u8]) -> Result<()> {
    if manifest_bytes.len() < MANIFEST_HEADER_SIZE {
        return Err(StorkvError::Corruption("Manifest header too short".into()));
    }
    if &manifest_bytes[0..8] != MANIFEST_MAGIC {
        return Err(StorkvError::Corruption("Invalid Manifest magic".into()));
    }
    Ok(())
}

fn replay_manifest(mut remaining_bytes: &[u8]) -> Result<ManifestState> {
    let mut state = ManifestState::default();
    while !remaining_bytes.is_empty() {
        let (edit, consumed_bytes) = VersionEdit::decode(remaining_bytes)?;
        state.apply(&edit);
        remaining_bytes = &remaining_bytes[consumed_bytes..];
    }
    Ok(state)
}

fn manifest_header() -> Vec<u8> {
    let mut header = BytesMut::with_capacity(MANIFEST_HEADER_SIZE);
    header.extend_from_slice(MANIFEST_MAGIC);
    header.put_u16_le(MANIFEST_VERSION);
    header.put_u16_le(0);
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32;
    header.put_u32_le(now_secs);
    header.to_vec()
}

fn activate_current<FS: FileSystem>(
    fs: &FS,
    db_root: &Path,
    current_path: &Path,
    manifest_filename: &str,
) -> Result<()> {
    let temp_current_path = db_root.join(CURRENT_TEMP_FILENAME);
    {
        let mut temp_file = fs.create_file(&temp_current_path)?;
        temp_file.write_all(format!("{}\n", manifest_filename).as_bytes())?;
        temp_file.sync_all()?;
    }
    fs.rename(&temp_current_path, current_path)?;
    fs.sync_dir(db_root)?;
    Ok(())
}
