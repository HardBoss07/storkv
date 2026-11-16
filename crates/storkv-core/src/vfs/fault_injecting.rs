use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::error::{Result, StorKvError};

use super::traits::{File, FileSystem};

/// Fault injector for chaos, crash-recovery, and torn-write testing.
pub struct FaultInjectingFs<FS: FileSystem> {
    inner: FS,
    fail_writes: AtomicBool,
    fail_syncs: AtomicBool,
    write_byte_limit: AtomicUsize,
    written_bytes: AtomicUsize,
}

impl<FS: FileSystem> FaultInjectingFs<FS> {
    /// Wrap a filesystem with the fault injector disabled.
    pub fn new(inner: FS) -> Self {
        Self {
            inner,
            fail_writes: AtomicBool::new(false),
            fail_syncs: AtomicBool::new(false),
            write_byte_limit: AtomicUsize::new(usize::MAX),
            written_bytes: AtomicUsize::new(0),
        }
    }

    /// Make every subsequent write fail when enabled.
    pub fn set_fail_writes(&self, fail: bool) {
        self.fail_writes.store(fail, Ordering::SeqCst);
    }

    /// Make every subsequent sync fail when enabled.
    pub fn set_fail_syncs(&self, fail: bool) {
        self.fail_syncs.store(fail, Ordering::SeqCst);
    }

    /// Make writes fail after the given number of bytes.
    pub fn set_write_byte_limit(&self, limit: usize) {
        self.write_byte_limit.store(limit, Ordering::SeqCst);
        self.written_bytes.store(0, Ordering::SeqCst);
    }

    fn wrap_file(&self, inner: Box<dyn File>) -> Box<dyn File> {
        Box::new(FaultInjectingFile {
            inner,
            fail_writes: Arc::new(AtomicBool::new(self.fail_writes.load(Ordering::SeqCst))),
            fail_syncs: Arc::new(AtomicBool::new(self.fail_syncs.load(Ordering::SeqCst))),
            write_byte_limit: Arc::new(AtomicUsize::new(
                self.write_byte_limit.load(Ordering::SeqCst),
            )),
            written_bytes: Arc::new(AtomicUsize::new(self.written_bytes.load(Ordering::SeqCst))),
        })
    }
}

/// File wrapper that applies the parent injector settings.
pub struct FaultInjectingFile {
    inner: Box<dyn File>,
    fail_writes: Arc<AtomicBool>,
    fail_syncs: Arc<AtomicBool>,
    write_byte_limit: Arc<AtomicUsize>,
    written_bytes: Arc<AtomicUsize>,
}

impl Read for FaultInjectingFile {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl Write for FaultInjectingFile {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("Injected write failure"));
        }

        let byte_limit = self.write_byte_limit.load(Ordering::SeqCst);
        let already_written = self.written_bytes.load(Ordering::SeqCst);
        if already_written >= byte_limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "Injected torn write: byte limit reached",
            ));
        }

        let allowable_bytes = byte_limit.saturating_sub(already_written).min(buffer.len());
        let bytes_written = self.inner.write(&buffer[..allowable_bytes])?;
        self.written_bytes
            .fetch_add(bytes_written, Ordering::SeqCst);
        Ok(bytes_written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for FaultInjectingFile {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(position)
    }
}

impl File for FaultInjectingFile {
    fn sync_all(&mut self) -> Result<()> {
        if self.fail_syncs.load(Ordering::SeqCst) {
            return Err(StorKvError::Io(std::io::Error::other(
                "Injected sync_all failure",
            )));
        }
        self.inner.sync_all()
    }

    fn sync_data(&mut self) -> Result<()> {
        if self.fail_syncs.load(Ordering::SeqCst) {
            return Err(StorKvError::Io(std::io::Error::other(
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
        let file = self.inner.create_file(path)?;
        Ok(self.wrap_file(file))
    }

    fn append_file(&self, path: &Path) -> Result<Box<dyn File>> {
        let file = self.inner.append_file(path)?;
        Ok(self.wrap_file(file))
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
