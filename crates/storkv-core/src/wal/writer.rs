use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::{BufMut, BytesMut};

use crate::error::Result;
use crate::vfs::{File, FileSystem};

use super::record::{Durability, WalBatchFrame};

/// Magic bytes at the start of every WAL segment.
pub const WAL_MAGIC: &[u8; 8] = b"STORKWAL";
/// Size of the WAL segment header in bytes.
pub const WAL_HEADER_SIZE: usize = 16;
/// Maximum size of a WAL segment before rotation.
pub const MAX_WAL_SEGMENT_BYTES: u64 = 32 * 1024 * 1024; // 32 MiB
/// Maximum age of a WAL segment before rotation.
pub const WAL_SEAL_INTERVAL: Duration = Duration::from_secs(300); // 5 minutes

const SEALED_FLAG_OFFSET: u64 = 10;
const SEALED_FLAG_SET: [u8; 2] = 1u16.to_le_bytes();
/// Version number written into every segment header.
const WAL_FORMAT_VERSION: u16 = 1;
/// Reserved header field, always zero in this format version.
const WAL_HEADER_RESERVED: u16 = 0;

/// One open WAL segment file.
pub struct WalSegment<FS: FileSystem> {
    // Fields ascend by binary width; the generic fields are dropped last.
    /// Segment file number.
    pub file_num: u64,
    /// Bytes written to the segment, including the header.
    pub written_bytes: u64,
    /// Monotonic time when the segment was created or reopened.
    pub created_at: Instant,
    /// Filesystem path of the segment.
    pub path: PathBuf,
    /// Open file handle.
    pub file: Box<dyn File>,
    _fs: Arc<FS>,
}

const _: () = assert!(std::mem::size_of::<WalBatchFrame>() <= 32);

impl<FS: FileSystem> WalSegment<FS> {
    /// Create a new segment and write its header.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be created or synced.
    pub fn create(fs: Arc<FS>, wal_dir: &Path, file_num: u64) -> Result<Self> {
        let path = segment_path(wal_dir, file_num);
        let mut file = fs.create_file(&path)?;
        file.write_all(&segment_header())?;
        file.sync_all()?;

        Ok(Self {
            file_num,
            path,
            file,
            written_bytes: WAL_HEADER_SIZE as u64,
            created_at: Instant::now(),
            _fs: fs,
        })
    }

    /// Attach to an existing segment instead of truncating it.
    ///
    /// Returns `Ok(None)` when the file is missing, too short to hold a header,
    /// or does not carry the StorKV WAL magic; the caller then creates a fresh
    /// segment. Reopening a database must never discard the active WAL tail
    /// before recovery has had a chance to replay it.
    pub fn open_existing(fs: Arc<FS>, wal_dir: &Path, file_num: u64) -> Result<Option<Self>> {
        let path = segment_path(wal_dir, file_num);
        if !fs.exists(&path) {
            return Ok(None);
        }
        let mut file = fs.append_file(&path)?;
        let size = file.file_size()?;
        if size < WAL_HEADER_SIZE as u64 {
            return Ok(None);
        }
        file.seek(std::io::SeekFrom::Start(0))?;
        let mut header = [0u8; WAL_HEADER_SIZE];
        if file.read_exact(&mut header).is_err() || &header[0..8] != WAL_MAGIC {
            return Ok(None);
        }
        file.seek(std::io::SeekFrom::End(0))?;
        Ok(Some(Self {
            file_num,
            path,
            file,
            written_bytes: size,
            created_at: Instant::now(),
            _fs: fs,
        }))
    }

    /// Append one encoded batch to the segment.
    ///
    /// # Errors
    ///
    /// Returns an error when the frame cannot be encoded or written.
    pub fn write_frame(&mut self, frame: &WalBatchFrame) -> Result<()> {
        let frame_bytes = frame.encode(false)?;
        self.file.write_all(&frame_bytes)?;
        self.written_bytes += frame_bytes.len() as u64;
        Ok(())
    }

    /// Sync the segment data to stable storage.
    ///
    /// # Errors
    ///
    /// Returns an error when the sync fails.
    pub fn sync(&mut self) -> Result<()> {
        self.file.sync_data()
    }

    /// Return true when the segment reached its size or age limit.
    pub fn should_rotate(&self) -> bool {
        self.written_bytes >= MAX_WAL_SEGMENT_BYTES
            || self.created_at.elapsed() >= WAL_SEAL_INTERVAL
    }

    /// Set the sealed flag and sync the segment header.
    ///
    /// # Errors
    ///
    /// Returns an error when the seek, write, or sync fails.
    pub fn seal(&mut self) -> Result<()> {
        self.file
            .seek(std::io::SeekFrom::Start(SEALED_FLAG_OFFSET))?;
        self.file.write_all(&SEALED_FLAG_SET)?;
        self.file.sync_all()
    }
}

/// Owns the active WAL segment and rotates it on size or age limits.
pub struct WalManager<FS: FileSystem> {
    fs: Arc<FS>,
    wal_dir: PathBuf,
    active_segment: parking_lot::Mutex<WalSegment<FS>>,
    next_wal_num: AtomicU64,
}

impl<FS: FileSystem> WalManager<FS> {
    /// Open or create the active segment.
    ///
    /// # Errors
    ///
    /// Returns an error when the WAL directory or segment cannot be opened.
    pub fn open(fs: Arc<FS>, wal_dir: &Path, active_wal_num: u64) -> Result<Self> {
        fs.create_dir_all(wal_dir)?;
        let active_segment = match WalSegment::open_existing(fs.clone(), wal_dir, active_wal_num)? {
            Some(segment) => segment,
            None => WalSegment::create(fs.clone(), wal_dir, active_wal_num)?,
        };
        Ok(Self {
            fs,
            wal_dir: wal_dir.to_path_buf(),
            active_segment: parking_lot::Mutex::new(active_segment),
            next_wal_num: AtomicU64::new(active_wal_num + 1),
        })
    }

    /// Write one batch and sync according to the durability policy.
    ///
    /// # Errors
    ///
    /// Returns an error when rotation, writing, or syncing fails.
    pub fn write_batch(&self, frame: &WalBatchFrame, durability: Durability) -> Result<()> {
        let mut segment = self.active_segment.lock();
        self.rotate_if_needed(&mut segment)?;
        segment.write_frame(frame)?;

        if matches!(durability, Durability::StrictFsync) {
            segment.sync()?;
        }
        Ok(())
    }

    /// Sync the active segment.
    ///
    /// # Errors
    ///
    /// Returns an error when the sync fails.
    pub fn sync_active(&self) -> Result<()> {
        let mut segment = self.active_segment.lock();
        segment.sync()
    }

    /// Return the file number of the active segment.
    pub fn current_file_num(&self) -> u64 {
        self.active_segment.lock().file_num
    }

    /// Return the write offset of the active segment.
    pub fn current_offset(&self) -> u64 {
        self.active_segment.lock().written_bytes
    }

    fn rotate_if_needed(&self, segment: &mut WalSegment<FS>) -> Result<()> {
        if !segment.should_rotate() {
            return Ok(());
        }
        segment.seal()?;
        let new_wal_num = self.next_wal_num.fetch_add(1, Ordering::SeqCst);
        let new_segment = WalSegment::create(self.fs.clone(), &self.wal_dir, new_wal_num)?;
        *segment = new_segment;
        Ok(())
    }
}

fn segment_path(wal_dir: &Path, file_num: u64) -> PathBuf {
    wal_dir.join(format!("{:016x}.wal", file_num))
}

fn segment_header() -> BytesMut {
    let mut header = BytesMut::with_capacity(WAL_HEADER_SIZE);
    header.extend_from_slice(WAL_MAGIC);
    header.put_u16_le(WAL_FORMAT_VERSION);
    header.put_u16_le(WAL_HEADER_RESERVED);
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32;
    header.put_u32_le(now_secs);
    header
}
