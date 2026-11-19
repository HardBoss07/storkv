use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::error::{Result, StorkvError};

use super::traits::{File, FileSystem};

/// In-RAM filesystem backend.
///
/// Every clone shares the same buffers. The backend supports directories,
/// files, append, truncate, rename, hard links, and no-op syncs.
#[derive(Clone, Default)]
pub struct InMemoryFs {
    state: Arc<RwLock<MemoryState>>,
}

#[derive(Default)]
struct MemoryState {
    directories: HashSet<PathBuf>,
    files: HashMap<PathBuf, Arc<RwLock<Vec<u8>>>>,
}

impl InMemoryFs {
    /// Create an empty in-memory filesystem.
    pub fn new() -> Self {
        Self::default()
    }

    fn file_buffer(&self, path: &Path) -> Result<Arc<RwLock<Vec<u8>>>> {
        self.state
            .read()
            .files
            .get(path)
            .cloned()
            .ok_or_else(|| missing_file(path))
    }

    fn insert_file(&self, path: &Path, buffer: Arc<RwLock<Vec<u8>>>) -> Box<dyn File> {
        self.state
            .write()
            .files
            .insert(path.to_path_buf(), buffer.clone());
        Box::new(InMemoryFile {
            buffer,
            position: 0,
        })
    }
}

impl FileSystem for InMemoryFs {
    fn open_file(&self, path: &Path) -> Result<Box<dyn File>> {
        let buffer = self.file_buffer(path)?;
        Ok(Box::new(InMemoryFile {
            buffer,
            position: 0,
        }))
    }

    fn create_file(&self, path: &Path) -> Result<Box<dyn File>> {
        let buffer = Arc::new(RwLock::new(Vec::new()));
        Ok(self.insert_file(path, buffer))
    }

    fn append_file(&self, path: &Path) -> Result<Box<dyn File>> {
        let buffer = match self.file_buffer(path) {
            Ok(buffer) => buffer,
            Err(_) => Arc::new(RwLock::new(Vec::new())),
        };
        let position = buffer.read().len() as u64;
        self.state
            .write()
            .files
            .insert(path.to_path_buf(), buffer.clone());
        Ok(Box::new(InMemoryFile { buffer, position }))
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        let mut directories = self.state.write();
        for ancestor in path.ancestors() {
            directories.directories.insert(ancestor.to_path_buf());
        }
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        self.state.write().files.remove(path);
        Ok(())
    }

    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        let mut state = self.state.write();
        state
            .files
            .retain(|candidate, _| !candidate.starts_with(path));
        state
            .directories
            .retain(|candidate| !candidate.starts_with(path));
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let mut state = self.state.write();
        let moved: Vec<(PathBuf, _)> = state
            .files
            .iter()
            .filter(|(candidate, _)| candidate.starts_with(from))
            .map(|(candidate, buffer)| {
                let suffix = candidate.strip_prefix(from).unwrap_or(Path::new(""));
                (to.join(suffix), buffer.clone())
            })
            .collect();
        state
            .files
            .retain(|candidate, _| !candidate.starts_with(from));
        for (candidate, buffer) in moved {
            state.files.insert(candidate, buffer);
        }

        let moved_dirs: Vec<PathBuf> = state
            .directories
            .iter()
            .filter(|candidate| candidate.starts_with(from))
            .map(|candidate| {
                let suffix = candidate.strip_prefix(from).unwrap_or(Path::new(""));
                to.join(suffix)
            })
            .collect();
        state
            .directories
            .retain(|candidate| !candidate.starts_with(from));
        for candidate in moved_dirs {
            state.directories.insert(candidate);
        }
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        let state = self.state.read();
        state.files.contains_key(path) || state.directories.contains(path)
    }

    fn hard_link(&self, src: &Path, dst: &Path) -> Result<()> {
        let buffer = self.file_buffer(src)?;
        self.state.write().files.insert(dst.to_path_buf(), buffer);
        Ok(())
    }

    fn sync_dir(&self, _path: &Path) -> Result<()> {
        Ok(())
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let state = self.state.read();
        let mut entries: Vec<PathBuf> = state
            .files
            .keys()
            .chain(state.directories.iter())
            .filter(|candidate| candidate.parent() == Some(path))
            .cloned()
            .collect();
        entries.sort();
        entries.dedup();
        Ok(entries)
    }

    fn file_size(&self, path: &Path) -> Result<u64> {
        Ok(self.file_buffer(path)?.read().len() as u64)
    }
}

/// Open handle to one in-memory buffer.
pub struct InMemoryFile {
    buffer: Arc<RwLock<Vec<u8>>>,
    position: u64,
}

impl Read for InMemoryFile {
    fn read(&mut self, destination: &mut [u8]) -> std::io::Result<usize> {
        let buffer = self.buffer.read();
        let start = usize::try_from(self.position)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        let available = &buffer[start..];
        let count = available.len().min(destination.len());
        destination[..count].copy_from_slice(&available[..count]);
        self.position = self.position.saturating_add(count as u64);
        Ok(count)
    }
}

impl Write for InMemoryFile {
    fn write(&mut self, source: &[u8]) -> std::io::Result<usize> {
        let mut buffer = self.buffer.write();
        let start = usize::try_from(self.position)
            .map_err(|_| std::io::Error::other("in-memory file position overflow"))?;
        let end = start
            .checked_add(source.len())
            .ok_or_else(|| std::io::Error::other("in-memory file write overflow"))?;
        if buffer.len() < end {
            buffer.resize(end, 0);
        }
        buffer[start..end].copy_from_slice(source);
        self.position = end as u64;
        Ok(source.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for InMemoryFile {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let buffer_len = self.buffer.read().len() as i128;
        let current = self.position as i128;
        let target = match position {
            SeekFrom::Start(offset) => offset as i128,
            SeekFrom::End(offset) => buffer_len + offset as i128,
            SeekFrom::Current(offset) => current + offset as i128,
        };
        if target < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "negative seek position",
            ));
        }
        self.position =
            u64::try_from(target).map_err(|_| std::io::Error::other("seek position overflow"))?;
        Ok(self.position)
    }
}

impl File for InMemoryFile {
    fn sync_all(&mut self) -> Result<()> {
        Ok(())
    }

    fn sync_data(&mut self) -> Result<()> {
        Ok(())
    }

    fn set_len(&mut self, size: u64) -> Result<()> {
        let length = usize::try_from(size)
            .map_err(|_| StorkvError::InvalidOperation("in-memory file too large".into()))?;
        self.buffer.write().resize(length, 0);
        Ok(())
    }

    fn file_size(&mut self) -> Result<u64> {
        Ok(self.buffer.read().len() as u64)
    }
}

fn missing_file(path: &Path) -> StorkvError {
    StorkvError::Io(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("in-memory file not found: {}", path.display()),
    ))
}
