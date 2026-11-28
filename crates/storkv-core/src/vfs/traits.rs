//! Virtual filesystem traits and shared types.
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::Result;

/// Open file handle used by the storage engine.
pub trait File: Read + Write + Seek + Send + Sync {
    /// Sync file data and metadata to stable storage.
    fn sync_all(&mut self) -> Result<()>;
    /// Sync file data to stable storage.
    fn sync_data(&mut self) -> Result<()>;
    /// Truncate or extend the file to the given size.
    fn set_len(&mut self, size: u64) -> Result<()>;
    /// Return the current file size in bytes.
    fn file_size(&mut self) -> Result<u64>;
}

/// Filesystem operations used by the storage engine.
///
/// Implementations must be synchronous and thread safe. The trait exists so
/// tests can inject faults without touching the real disk.
pub trait FileSystem: Send + Sync {
    /// Open an existing file for reading.
    fn open_file(&self, path: &Path) -> Result<Box<dyn File>>;
    /// Create or truncate a file for writing.
    fn create_file(&self, path: &Path) -> Result<Box<dyn File>>;
    /// Open a file for appending.
    fn append_file(&self, path: &Path) -> Result<Box<dyn File>>;
    /// Create a directory and its parents.
    fn create_dir_all(&self, path: &Path) -> Result<()>;
    /// Delete one file.
    fn remove_file(&self, path: &Path) -> Result<()>;
    /// Delete a directory and its contents.
    fn remove_dir_all(&self, path: &Path) -> Result<()>;
    /// Rename a file or directory.
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    /// Return true when the path exists.
    fn exists(&self, path: &Path) -> bool;
    /// Create a hard link.
    fn hard_link(&self, src: &Path, dst: &Path) -> Result<()>;
    /// Sync a directory entry to stable storage.
    fn sync_dir(&self, path: &Path) -> Result<()>;
    /// List the entries of a directory.
    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>>;
    /// Return the size of a file in bytes.
    fn file_size(&self, path: &Path) -> Result<u64>;
    /// Return a fresh in-memory view of this filesystem.
    ///
    /// The default implementation returns `None`. Selectors override this
    /// method to give one scope an in-RAM backend.
    fn memory_view(&self) -> Option<Arc<Self>>
    where
        Self: Sized,
    {
        None
    }
}
