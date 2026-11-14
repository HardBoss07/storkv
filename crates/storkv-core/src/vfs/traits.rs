use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use crate::error::Result;

pub trait File: Read + Write + Seek + Send + Sync {
    fn sync_all(&mut self) -> Result<()>;
    fn sync_data(&mut self) -> Result<()>;
    fn set_len(&mut self, size: u64) -> Result<()>;
    fn file_size(&mut self) -> Result<u64>;
}

pub trait FileSystem: Send + Sync {
    fn open_file(&self, path: &Path) -> Result<Box<dyn File>>;
    fn create_file(&self, path: &Path) -> Result<Box<dyn File>>;
    fn append_file(&self, path: &Path) -> Result<Box<dyn File>>;
    fn create_dir_all(&self, path: &Path) -> Result<()>;
    fn remove_file(&self, path: &Path) -> Result<()>;
    fn remove_dir_all(&self, path: &Path) -> Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    fn exists(&self, path: &Path) -> bool;
    fn hard_link(&self, src: &Path, dst: &Path) -> Result<()>;
    fn sync_dir(&self, path: &Path) -> Result<()>;
    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>>;
    fn file_size(&self, path: &Path) -> Result<u64>;
}
