use crate::error::Result;
use std::fs::File as StdFile;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

pub struct PosixFile {
    inner: StdFile,
}

impl PosixFile {
    pub fn new(inner: StdFile) -> Self {
        Self { inner }
    }
}

impl Read for PosixFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Write for PosixFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for PosixFile {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
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
        let meta = self.inner.metadata()?;
        Ok(meta.len())
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
        // On Unix, opening the directory and calling fsync is standard.
        // On Windows opening a directory as a file might fail or be a no-op, so handle cleanly.
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
        let mut results = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            results.push(entry.path());
        }
        Ok(results)
    }

    fn file_size(&self, path: &Path) -> Result<u64> {
        let meta = std::fs::metadata(path)?;
        Ok(meta.len())
    }
}

/// Fault injector for chaos, crash-recovery, and torn-write testing.
pub struct FaultInjectingFs<FS: FileSystem> {
    inner: FS,
    fail_writes: AtomicBool,
    fail_syncs: AtomicBool,
    write_byte_limit: AtomicUsize,
    written_bytes: AtomicUsize,
}

impl<FS: FileSystem> FaultInjectingFs<FS> {
    pub fn new(inner: FS) -> Self {
        Self {
            inner,
            fail_writes: AtomicBool::new(false),
            fail_syncs: AtomicBool::new(false),
            write_byte_limit: AtomicUsize::new(usize::MAX),
            written_bytes: AtomicUsize::new(0),
        }
    }

    pub fn set_fail_writes(&self, fail: bool) {
        self.fail_writes.store(fail, Ordering::SeqCst);
    }

    pub fn set_fail_syncs(&self, fail: bool) {
        self.fail_syncs.store(fail, Ordering::SeqCst);
    }

    pub fn set_write_byte_limit(&self, limit: usize) {
        self.write_byte_limit.store(limit, Ordering::SeqCst);
        self.written_bytes.store(0, Ordering::SeqCst);
    }
}

pub struct FaultInjectingFile {
    inner: Box<dyn File>,
    fail_writes: Arc<AtomicBool>,
    fail_syncs: Arc<AtomicBool>,
    write_byte_limit: Arc<AtomicUsize>,
    written_bytes: Arc<AtomicUsize>,
}

impl Read for FaultInjectingFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Write for FaultInjectingFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("Injected write failure"));
        }

        let limit = self.write_byte_limit.load(Ordering::SeqCst);
        let curr = self.written_bytes.load(Ordering::SeqCst);
        if curr >= limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "Injected torn write: byte limit reached",
            ));
        }

        let allowable = limit.saturating_sub(curr).min(buf.len());
        let n = self.inner.write(&buf[..allowable])?;
        self.written_bytes.fetch_add(n, Ordering::SeqCst);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for FaultInjectingFile {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl File for FaultInjectingFile {
    fn sync_all(&mut self) -> Result<()> {
        if self.fail_syncs.load(Ordering::SeqCst) {
            return Err(crate::error::StorKvError::Io(std::io::Error::other(
                "Injected sync_all failure",
            )));
        }
        self.inner.sync_all()
    }

    fn sync_data(&mut self) -> Result<()> {
        if self.fail_syncs.load(Ordering::SeqCst) {
            return Err(crate::error::StorKvError::Io(std::io::Error::other(
                "Injected sync_data failure",
            )));
        }
        self.inner.sync_data()
    }

    fn set_len(&mut self, size: u64) -> Result<()> {
        self.inner.set_len(size)
    }

    fn file_size(&mut self) -> Result<u64> {
        self.inner.file_size()
    }
}

impl<FS: FileSystem> FileSystem for Arc<FaultInjectingFs<FS>> {
    fn open_file(&self, path: &Path) -> Result<Box<dyn File>> {
        self.inner.open_file(path)
    }

    fn create_file(&self, path: &Path) -> Result<Box<dyn File>> {
        let f = self.inner.create_file(path)?;
        Ok(Box::new(FaultInjectingFile {
            inner: f,
            fail_writes: Arc::new(AtomicBool::new(self.fail_writes.load(Ordering::SeqCst))),
            fail_syncs: Arc::new(AtomicBool::new(self.fail_syncs.load(Ordering::SeqCst))),
            write_byte_limit: Arc::new(AtomicUsize::new(
                self.write_byte_limit.load(Ordering::SeqCst),
            )),
            written_bytes: Arc::new(AtomicUsize::new(self.written_bytes.load(Ordering::SeqCst))),
        }))
    }

    fn append_file(&self, path: &Path) -> Result<Box<dyn File>> {
        let f = self.inner.append_file(path)?;
        Ok(Box::new(FaultInjectingFile {
            inner: f,
            fail_writes: Arc::new(AtomicBool::new(self.fail_writes.load(Ordering::SeqCst))),
            fail_syncs: Arc::new(AtomicBool::new(self.fail_syncs.load(Ordering::SeqCst))),
            write_byte_limit: Arc::new(AtomicUsize::new(
                self.write_byte_limit.load(Ordering::SeqCst),
            )),
            written_bytes: Arc::new(AtomicUsize::new(self.written_bytes.load(Ordering::SeqCst))),
        }))
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        self.inner.create_dir_all(path)
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        self.inner.remove_file(path)
    }

    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        self.inner.remove_dir_all(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        self.inner.rename(from, to)
    }

    fn exists(&self, path: &Path) -> bool {
        self.inner.exists(path)
    }

    fn hard_link(&self, src: &Path, dst: &Path) -> Result<()> {
        self.inner.hard_link(src, dst)
    }

    fn sync_dir(&self, path: &Path) -> Result<()> {
        self.inner.sync_dir(path)
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>> {
        self.inner.read_dir(path)
    }

    fn file_size(&self, path: &Path) -> Result<u64> {
        self.inner.file_size(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_posix_fs_roundtrip() {
        let temp = std::env::temp_dir().join(format!("storkv_test_vfs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let fs = PosixFs;
        fs.create_dir_all(&temp).unwrap();
        let file_path = temp.join("test.txt");

        {
            let mut file = fs.create_file(&file_path).unwrap();
            file.write_all(b"hello world").unwrap();
            file.sync_all().unwrap();
            assert_eq!(file.file_size().unwrap(), 11);
        }

        {
            let mut file = fs.open_file(&file_path).unwrap();
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).unwrap();
            assert_eq!(buf, b"hello world");
        }

        let link_path = temp.join("link.txt");
        // On Windows hard_link across or in temp might require privileges or fail, handle or test copy
        let _ = fs.hard_link(&file_path, &link_path);

        let _ = fs.remove_dir_all(&temp);
    }
}
