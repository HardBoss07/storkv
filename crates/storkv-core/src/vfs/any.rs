//! Runtime-selected virtual filesystem wrapper.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::Result;

use super::memory::InMemoryFs;
use super::traits::{File, FileSystem};

/// Filesystem selector that routes each operation to disk or RAM.
///
/// The database root uses the selected variant. A single database can route
/// individual scopes to the memory variant for per-scope residency.
pub enum AnyFs<FS: FileSystem> {
    /// Forward every call to the wrapped disk filesystem.
    Disk(FS),
    /// Keep every file in RAM.
    Memory(InMemoryFs),
}

impl<FS: FileSystem> AnyFs<FS> {
    /// Return true when the selector uses the memory backend.
    pub fn is_memory(&self) -> bool {
        matches!(self, AnyFs::Memory(_))
    }
}

impl<FS: FileSystem> FileSystem for AnyFs<FS> {
    fn open_file(&self, path: &Path) -> Result<Box<dyn File>> {
        match self {
            AnyFs::Disk(inner) => inner.open_file(path),
            AnyFs::Memory(inner) => inner.open_file(path),
        }
    }

    fn create_file(&self, path: &Path) -> Result<Box<dyn File>> {
        match self {
            AnyFs::Disk(inner) => inner.create_file(path),
            AnyFs::Memory(inner) => inner.create_file(path),
        }
    }

    fn append_file(&self, path: &Path) -> Result<Box<dyn File>> {
        match self {
            AnyFs::Disk(inner) => inner.append_file(path),
            AnyFs::Memory(inner) => inner.append_file(path),
        }
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        match self {
            AnyFs::Disk(inner) => inner.create_dir_all(path),
            AnyFs::Memory(inner) => inner.create_dir_all(path),
        }
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        match self {
            AnyFs::Disk(inner) => inner.remove_file(path),
            AnyFs::Memory(inner) => inner.remove_file(path),
        }
    }

    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        match self {
            AnyFs::Disk(inner) => inner.remove_dir_all(path),
            AnyFs::Memory(inner) => inner.remove_dir_all(path),
        }
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        match self {
            AnyFs::Disk(inner) => inner.rename(from, to),
            AnyFs::Memory(inner) => inner.rename(from, to),
        }
    }

    fn exists(&self, path: &Path) -> bool {
        match self {
            AnyFs::Disk(inner) => inner.exists(path),
            AnyFs::Memory(inner) => inner.exists(path),
        }
    }

    fn hard_link(&self, src: &Path, dst: &Path) -> Result<()> {
        match self {
            AnyFs::Disk(inner) => inner.hard_link(src, dst),
            AnyFs::Memory(inner) => inner.hard_link(src, dst),
        }
    }

    fn sync_dir(&self, path: &Path) -> Result<()> {
        match self {
            AnyFs::Disk(inner) => inner.sync_dir(path),
            AnyFs::Memory(inner) => inner.sync_dir(path),
        }
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>> {
        match self {
            AnyFs::Disk(inner) => inner.read_dir(path),
            AnyFs::Memory(inner) => inner.read_dir(path),
        }
    }

    fn file_size(&self, path: &Path) -> Result<u64> {
        match self {
            AnyFs::Disk(inner) => inner.file_size(path),
            AnyFs::Memory(inner) => inner.file_size(path),
        }
    }

    fn memory_view(&self) -> Option<Arc<Self>> {
        match self {
            AnyFs::Disk(_) => Some(Arc::new(AnyFs::Memory(InMemoryFs::new()))),
            AnyFs::Memory(_) => Some(Arc::new(AnyFs::Memory(InMemoryFs::new()))),
        }
    }
}
