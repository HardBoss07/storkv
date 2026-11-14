use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::{BufMut, BytesMut};

use crate::error::Result;
use crate::vfs::{File, FileSystem};

use super::record::{Durability, WalBatchFrame};

pub const WAL_MAGIC: &[u8; 8] = b"STORKWAL";
pub const WAL_HEADER_SIZE: usize = 16;
pub const MAX_WAL_SEGMENT_BYTES: u64 = 32 * 1024 * 1024; // 32 MiB
pub const WAL_SEAL_INTERVAL: Duration = Duration::from_secs(300); // 5 minutes

const SEALED_FLAG_OFFSET: u64 = 10;
const SEALED_FLAG_SET: [u8; 2] = 1u16.to_le_bytes();

pub struct WalSegment<FS: FileSystem> {
    // Widest fields first; the generic fields are dropped last.
    pub file_num: u64,
    pub written_bytes: u64,
    pub created_at: Instant,
    pub path: PathBuf,
    pub file: Box<dyn File>,
    _fs: Arc<FS>,
}

const _: () = assert!(std::mem::size_of::<WalBatchFrame>() <= 32);

impl<FS: FileSystem> WalSegment<FS> {
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

    pub fn write_frame(&mut self, frame: &WalBatchFrame) -> Result<()> {
        let frame_bytes = frame.encode(false)?;
        self.file.write_all(&frame_bytes)?;
        self.written_bytes += frame_bytes.len() as u64;
        Ok(())
    }

    pub fn sync(&mut self) -> Result<()> {
        self.file.sync_data()
    }

    pub fn should_rotate(&self) -> bool {
        self.written_bytes >= MAX_WAL_SEGMENT_BYTES
            || self.created_at.elapsed() >= WAL_SEAL_INTERVAL
    }

    pub fn seal(&mut self) -> Result<()> {
        self.file
            .seek(std::io::SeekFrom::Start(SEALED_FLAG_OFFSET))?;
        self.file.write_all(&SEALED_FLAG_SET)?;
        self.file.sync_all()
    }
}

pub struct WalManager<FS: FileSystem> {
    fs: Arc<FS>,
    wal_dir: PathBuf,
    active_segment: parking_lot::Mutex<WalSegment<FS>>,
    next_wal_num: AtomicU64,
}

impl<FS: FileSystem> WalManager<FS> {
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

    pub fn write_batch(&self, frame: &WalBatchFrame, durability: Durability) -> Result<()> {
        let mut segment = self.active_segment.lock();
        self.rotate_if_needed(&mut segment)?;
        segment.write_frame(frame)?;

        if matches!(durability, Durability::StrictFsync) {
            segment.sync()?;
        }
        Ok(())
    }

    pub fn sync_active(&self) -> Result<()> {
        let mut segment = self.active_segment.lock();
        segment.sync()
    }

    pub fn current_file_num(&self) -> u64 {
        self.active_segment.lock().file_num
    }

    pub fn current_offset(&self) -> u64 {
        self.active_segment.lock().written_bytes
    }

    fn rotate_if_needed(&self, segment: &mut WalSegment<FS>) -> Result<()> {
        if !segment.should_rotate() {
            return Ok(());
        }
        let _ = segment.seal();
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
    header.put_u16_le(1);
    header.put_u16_le(0);
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32;
    header.put_u32_le(now_secs);
    header
}
