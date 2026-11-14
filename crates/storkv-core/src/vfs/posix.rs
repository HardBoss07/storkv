use std::fs::File as StdFile;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::Result;

use super::traits::{File, FileSystem};

pub struct PosixFile {
    inner: StdFile,
}

impl PosixFile {
    pub fn new(inner: StdFile) -> Self {
        Self { inner }
    }
}

impl Read for PosixFile {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl Write for PosixFile {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for PosixFile {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(position)
    }
}

impl File for PosixFile {
    fn sync_all(&mut self) -> Result<()> {
        self.inner.sync_all()?;
        Ok(())
    }

    fn sync_data(&mut self) -> Result<()> {
        self.inner.sync_data()?;
        Ok(())
    }

    fn set_len(&mut self, size: u64) -> Result<()> {
        self.inner.set_len(size)?;
        Ok(())
    }

    fn file_size(&mut self) -> Result<u64> {
        let metadata = self.inner.metadata()?;
        Ok(metadata.len())
    }
}

#[derive(Default, Clone)]
pub struct PosixFs;

impl FileSystem for PosixFs {
    fn open_file(&self, path: &Path) -> Result<Box<dyn File>> {
        let file = std::fs::OpenOptions::new().read(true).open(path)?;
        Ok(Box::new(PosixFile::new(file)))
    }

    fn create_file(&self, path: &Path) -> Result<Box<dyn File>> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        Ok(Box::new(PosixFile::new(file)))
    }

    fn append_file(&self, path: &Path) -> Result<Box<dyn File>> {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;
        file.seek(SeekFrom::End(0))?;
        Ok(Box::new(PosixFile::new(file)))
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        std::fs::create_dir_all(path)?;
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        if path.exists() {
            std::fs::remove_dir_all(path)?;
        }
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        std::fs::rename(from, to)?;
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn hard_link(&self, src: &Path, dst: &Path) -> Result<()> {
        std::fs::hard_link(src, dst)?;
        Ok(())
    }

    fn sync_dir(&self, path: &Path) -> Result<()> {
        #[cfg(unix)]
        {
            let dir = StdFile::open(path)?;
            dir.sync_all()?;
        }
        #[cfg(not(unix))]
        {
            let _ = path;
        }
        Ok(())
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            paths.push(entry.path());
        }
        Ok(paths)
    }

    fn file_size(&self, path: &Path) -> Result<u64> {
        let metadata = std::fs::metadata(path)?;
        Ok(metadata.len())
    }
}
