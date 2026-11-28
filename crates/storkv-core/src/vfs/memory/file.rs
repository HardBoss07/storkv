//! Open handle to one in-memory file buffer.
//!
//! The handle shares its buffer with the parent [`super::InMemoryFs`]. Random
//! access uses a byte position, and writes extend the buffer with zeroes.

use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::error::{Result, StorkvError};

use super::super::traits::File;

/// Open handle to one in-memory buffer.
pub struct InMemoryFile {
    pub(super) buffer: Arc<RwLock<Vec<u8>>>,
    pub(super) position: u64,
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
