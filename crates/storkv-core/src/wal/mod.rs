use crate::error::{Result, StorKvError};
use crate::format::{MAX_KEY_LEN, MAX_VAL_LEN, compute_crc32c};
use crate::vfs::{File, FileSystem};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const WAL_MAGIC: &[u8; 8] = b"STORKWAL";
pub const WAL_HEADER_SIZE: usize = 16;
pub const WAL_FRAME_HEADER_SIZE: usize = 24;
pub const MAX_WAL_SEGMENT_BYTES: u64 = 32 * 1024 * 1024; // 32 MiB
pub const WAL_SEAL_INTERVAL: Duration = Duration::from_secs(300); // 5 minutes

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    StrictFsync,
    Buffered { sync_interval: Duration },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalOp {
    Put {
        scope_id: u32,
        expires_at_ms: u64,
        key: Bytes,
        val: Bytes,
    },
    Delete {
        scope_id: u32,
        key: Bytes,
    },
    Merge {
        scope_id: u32,
        key: Bytes,
        val: Bytes,
    },
    DeleteRange {
        scope_id: u32,
        start_key: Bytes,
        end_key: Bytes,
    },
    DropScope {
        scope_id: u32,
    },
}

impl WalOp {
    pub fn opcode(&self) -> u8 {
        match self {
            WalOp::Put { .. } => 0x01,
            WalOp::Delete { .. } => 0x02,
            WalOp::Merge { .. } => 0x03,
            WalOp::DeleteRange { .. } => 0x04,
            WalOp::DropScope { .. } => 0x05,
        }
    }

    pub fn encode(&self, buf: &mut BytesMut) -> Result<()> {
        match self {
            WalOp::Put {
                scope_id,
                expires_at_ms,
                key,
                val,
            } => {
                if key.len() > MAX_KEY_LEN {
                    return Err(StorKvError::KeyTooLarge {
                        max: MAX_KEY_LEN,
                        actual: key.len(),
                    });
                }
                if val.len() > MAX_VAL_LEN {
                    return Err(StorKvError::ValueTooLarge {
                        max: MAX_VAL_LEN,
                        actual: val.len(),
                    });
                }
                buf.put_u8(0x01);
                buf.put_u32_le(*scope_id);
                buf.put_u64_le(*expires_at_ms);
                buf.put_u32_le(key.len() as u32);
                buf.extend_from_slice(key);
                buf.put_u32_le(val.len() as u32);
                buf.extend_from_slice(val);
            }
            WalOp::Delete { scope_id, key } => {
                if key.len() > MAX_KEY_LEN {
                    return Err(StorKvError::KeyTooLarge {
                        max: MAX_KEY_LEN,
                        actual: key.len(),
                    });
                }
                buf.put_u8(0x02);
                buf.put_u32_le(*scope_id);
                buf.put_u64_le(0);
                buf.put_u32_le(key.len() as u32);
                buf.extend_from_slice(key);
                buf.put_u32_le(0);
            }
            WalOp::Merge { scope_id, key, val } => {
                if key.len() > MAX_KEY_LEN {
                    return Err(StorKvError::KeyTooLarge {
                        max: MAX_KEY_LEN,
                        actual: key.len(),
                    });
                }
                if val.len() > MAX_VAL_LEN {
                    return Err(StorKvError::ValueTooLarge {
                        max: MAX_VAL_LEN,
                        actual: val.len(),
                    });
                }
                buf.put_u8(0x03);
                buf.put_u32_le(*scope_id);
                buf.put_u64_le(0);
                buf.put_u32_le(key.len() as u32);
                buf.extend_from_slice(key);
                buf.put_u32_le(val.len() as u32);
                buf.extend_from_slice(val);
            }
            WalOp::DeleteRange {
                scope_id,
                start_key,
                end_key,
            } => {
                if start_key.len() > MAX_KEY_LEN {
                    return Err(StorKvError::KeyTooLarge {
                        max: MAX_KEY_LEN,
                        actual: start_key.len(),
                    });
                }
                if end_key.len() > MAX_KEY_LEN {
                    return Err(StorKvError::KeyTooLarge {
                        max: MAX_KEY_LEN,
                        actual: end_key.len(),
                    });
                }
                buf.put_u8(0x04);
                buf.put_u32_le(*scope_id);
                buf.put_u64_le(0);
                buf.put_u32_le(start_key.len() as u32);
                buf.extend_from_slice(start_key);
                buf.put_u32_le(end_key.len() as u32);
                buf.extend_from_slice(end_key);
            }
            WalOp::DropScope { scope_id } => {
                buf.put_u8(0x05);
                buf.put_u32_le(*scope_id);
                buf.put_u64_le(0);
                buf.put_u32_le(0);
                buf.put_u32_le(0);
            }
        }
        Ok(())
    }

    pub fn decode(buf: &mut impl Buf) -> Result<Self> {
        if buf.remaining() < 17 {
            return Err(StorKvError::Corruption("Incomplete WalOp header".into()));
        }
        let opcode = buf.get_u8();
        let scope_id = buf.get_u32_le();
        let expires_at_ms = buf.get_u64_le();
        let key_len = buf.get_u32_le() as usize;

        if buf.remaining() < key_len + 4 {
            return Err(StorKvError::Corruption("Incomplete WalOp key".into()));
        }
        let mut key_bytes = vec![0u8; key_len];
        buf.copy_to_slice(&mut key_bytes);
        let key = Bytes::from(key_bytes);

        let val_len = buf.get_u32_le() as usize;
        if buf.remaining() < val_len {
            return Err(StorKvError::Corruption("Incomplete WalOp val".into()));
        }
        let mut val_bytes = vec![0u8; val_len];
        buf.copy_to_slice(&mut val_bytes);
        let val = Bytes::from(val_bytes);

        match opcode {
            0x01 => Ok(WalOp::Put {
                scope_id,
                expires_at_ms,
                key,
                val,
            }),
            0x02 => Ok(WalOp::Delete { scope_id, key }),
            0x03 => Ok(WalOp::Merge { scope_id, key, val }),
            0x04 => Ok(WalOp::DeleteRange {
                scope_id,
                start_key: key,
                end_key: val,
            }),
            0x05 => Ok(WalOp::DropScope { scope_id }),
            _ => Err(StorKvError::Corruption(format!(
                "Unknown WalOp opcode: 0x{:02x}",
                opcode
            ))),
        }
    }
}

/// WAL batch frame:
/// +----------+-----------+-----------+----------+-------+----------+--------------------+
/// | crc32c   | frame_len | batch_seq | op_count | flags | reserved | payload            |
/// | (u32 LE) | (u32 LE)  | (u64 LE)  | (u32 LE) | (u16) | (u16)    | ([u8; ...])        |
/// +----------+-----------+-----------+----------+-------+----------+--------------------+
pub struct WalBatchFrame {
    pub batch_seq: u64,
    pub ops: Vec<WalOp>,
}

impl WalBatchFrame {
    pub fn new(batch_seq: u64, ops: Vec<WalOp>) -> Self {
        Self { batch_seq, ops }
    }

    pub fn encode(&self, compress: bool) -> Result<Vec<u8>> {
        let mut payload = BytesMut::new();
        for op in &self.ops {
            op.encode(&mut payload)?;
        }

        let (flags, final_payload) = if compress && payload.len() > 128 {
            let mut comp = BytesMut::new();
            comp.put_u32_le(payload.len() as u32);
            let compressed = lz4_flex::compress_prepend_size(&payload);
            comp.extend_from_slice(&compressed);
            (0x0001u16, comp.freeze())
        } else {
            (0x0000u16, payload.freeze())
        };

        // frame_len covers: batch_seq (8) + op_count (4) + flags (2) + reserved (2) + payload
        let frame_len = (8 + 4 + 2 + 2 + final_payload.len()) as u32;

        let mut data_for_crc = BytesMut::with_capacity(4 + frame_len as usize);
        data_for_crc.put_u32_le(frame_len);
        data_for_crc.put_u64_le(self.batch_seq);
        data_for_crc.put_u32_le(self.ops.len() as u32);
        data_for_crc.put_u16_le(flags);
        data_for_crc.put_u16_le(0); // reserved
        data_for_crc.extend_from_slice(&final_payload);

        let crc = compute_crc32c(&data_for_crc);

        let mut out = Vec::with_capacity(4 + data_for_crc.len());
        out.put_u32_le(crc);
        out.extend_from_slice(&data_for_crc);
        Ok(out)
    }

    /// Decode frame from byte buffer. Returns (frame, bytes_consumed)
    pub fn decode(buf: &[u8]) -> Result<(Self, usize)> {
        if buf.len() < WAL_FRAME_HEADER_SIZE {
            return Err(StorKvError::Corruption("Truncated WAL frame header".into()));
        }

        let expected_crc = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        let frame_len = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
        let total_frame_len = 4 + 4 + frame_len;

        if buf.len() < total_frame_len {
            return Err(StorKvError::Corruption(
                "Truncated WAL frame payload".into(),
            ));
        }

        let computed_crc = compute_crc32c(&buf[4..total_frame_len]);
        if expected_crc != computed_crc {
            return Err(StorKvError::Corruption(format!(
                "WAL frame CRC mismatch: expected 0x{:08x}, got 0x{:08x}",
                expected_crc, computed_crc
            )));
        }

        let mut cur = &buf[8..total_frame_len];
        let batch_seq = cur.get_u64_le();
        let op_count = cur.get_u32_le() as usize;
        let flags = cur.get_u16_le();
        let _reserved = cur.get_u16_le();

        let uncompressed_payload = if flags & 0x0001 != 0 {
            if cur.remaining() < 4 {
                return Err(StorKvError::Corruption(
                    "Truncated compressed WAL payload".into(),
                ));
            }
            let _uncomp_len = cur.get_u32_le();
            let mut remaining_bytes = vec![0u8; cur.remaining()];
            cur.copy_to_slice(&mut remaining_bytes);
            let decompressed = lz4_flex::decompress_size_prepended(&remaining_bytes)
                .map_err(|e| StorKvError::Corruption(e.to_string()))?;
            Bytes::from(decompressed)
        } else {
            let mut remaining_bytes = vec![0u8; cur.remaining()];
            cur.copy_to_slice(&mut remaining_bytes);
            Bytes::from(remaining_bytes)
        };

        let mut payload_buf = uncompressed_payload;
        let mut ops = Vec::with_capacity(op_count);
        for _ in 0..op_count {
            ops.push(WalOp::decode(&mut payload_buf)?);
        }

        Ok((WalBatchFrame { batch_seq, ops }, total_frame_len))
    }
}

pub struct WalSegment<FS: FileSystem> {
    pub file_num: u64,
    pub path: PathBuf,
    pub file: Box<dyn File>,
    pub written_bytes: u64,
    pub created_at: Instant,
    _fs: Arc<FS>,
}

impl<FS: FileSystem> WalSegment<FS> {
    pub fn create(fs: Arc<FS>, wal_dir: &Path, file_num: u64) -> Result<Self> {
        let filename = format!("{:016x}.wal", file_num);
        let path = wal_dir.join(filename);
        let mut file = fs.create_file(&path)?;

        let mut header = BytesMut::with_capacity(WAL_HEADER_SIZE);
        header.extend_from_slice(WAL_MAGIC);
        header.put_u16_le(1); // version 1
        header.put_u16_le(0); // flags: active
        let now_sec = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        header.put_u32_le(now_sec);

        file.write_all(&header)?;
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

    pub fn write_frame(&mut self, frame: &WalBatchFrame) -> Result<()> {
        let bytes = frame.encode(false)?;
        self.file.write_all(&bytes)?;
        self.written_bytes += bytes.len() as u64;
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
        // Mark flag bit 0x0001
        self.file.seek(std::io::SeekFrom::Start(10))?;
        self.file.write_all(&1u16.to_le_bytes())?;
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
        let seg = WalSegment::create(fs.clone(), wal_dir, active_wal_num)?;
        Ok(Self {
            fs,
            wal_dir: wal_dir.to_path_buf(),
            active_segment: parking_lot::Mutex::new(seg),
            next_wal_num: AtomicU64::new(active_wal_num + 1),
        })
    }

    pub fn write_batch(&self, frame: &WalBatchFrame, durability: Durability) -> Result<()> {
        let mut seg = self.active_segment.lock();
        if seg.should_rotate() {
            let _ = seg.seal();
            let new_num = self.next_wal_num.fetch_add(1, Ordering::SeqCst);
            let new_seg = WalSegment::create(self.fs.clone(), &self.wal_dir, new_num)?;
            *seg = new_seg;
        }

        seg.write_frame(frame)?;

        match durability {
            Durability::StrictFsync => {
                seg.sync()?;
            }
            Durability::Buffered { .. } => {
                // OS page cache write without immediate sync
            }
        }

        Ok(())
    }

    pub fn sync_active(&self) -> Result<()> {
        let mut seg = self.active_segment.lock();
        seg.sync()
    }

    pub fn current_file_num(&self) -> u64 {
        self.active_segment.lock().file_num
    }

    pub fn current_offset(&self) -> u64 {
        self.active_segment.lock().written_bytes
    }
}

/// Recovery reader for WAL segments:
/// Handles torn tail auto-truncation and mid-segment corruption errors.
pub struct WalRecovery;

impl WalRecovery {
    pub fn recover_segment<FS: FileSystem>(
        fs: Arc<FS>,
        path: &Path,
        is_last_segment: bool,
    ) -> Result<Vec<WalBatchFrame>> {
        let mut data = Vec::new();
        {
            let mut file = fs.open_file(path)?;
            file.read_to_end(&mut data)?;
        }

        if data.len() < WAL_HEADER_SIZE {
            if is_last_segment {
                // Truncate to empty/header
                return Ok(Vec::new());
            } else {
                return Err(StorKvError::Corruption(
                    "Truncated WAL segment header".into(),
                ));
            }
        }

        if &data[0..8] != WAL_MAGIC {
            return Err(StorKvError::Corruption("Invalid WAL magic".into()));
        }

        let mut frames = Vec::new();
        let mut offset = WAL_HEADER_SIZE;

        while offset < data.len() {
            match WalBatchFrame::decode(&data[offset..]) {
                Ok((frame, consumed)) => {
                    frames.push(frame);
                    offset += consumed;
                }
                Err(err) => {
                    if is_last_segment {
                        // Torn tail at the end of the last WAL segment: truncate and recover
                        let mut app_file = fs.append_file(path)?;
                        app_file.set_len(offset as u64)?;
                        app_file.sync_all()?;
                        break;
                    } else {
                        // Mid-segment corruption
                        return Err(err);
                    }
                }
            }
        }

        Ok(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::PosixFs;

    #[test]
    fn test_wal_frame_codec() {
        let ops = vec![
            WalOp::Put {
                scope_id: 1,
                expires_at_ms: 1000,
                key: Bytes::from_static(b"k1"),
                val: Bytes::from_static(b"v1"),
            },
            WalOp::Delete {
                scope_id: 1,
                key: Bytes::from_static(b"k2"),
            },
            WalOp::DropScope { scope_id: 2 },
        ];
        let frame = WalBatchFrame::new(100, ops);
        let bytes = frame.encode(false).unwrap();
        let (decoded, consumed) = WalBatchFrame::decode(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded.batch_seq, 100);
        assert_eq!(decoded.ops.len(), 3);
    }

    #[test]
    fn test_wal_recovery_torn_tail_vs_corruption() {
        let temp = std::env::temp_dir().join(format!("storkv_test_wal_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let fs = Arc::new(PosixFs);
        fs.create_dir_all(&temp).unwrap();

        let wal_mgr = WalManager::open(fs.clone(), &temp, 1).unwrap();
        let ops = vec![WalOp::Put {
            scope_id: 0,
            expires_at_ms: 0,
            key: Bytes::from_static(b"k"),
            val: Bytes::from_static(b"v"),
        }];
        wal_mgr
            .write_batch(&WalBatchFrame::new(1, ops), Durability::StrictFsync)
            .unwrap();
        drop(wal_mgr);

        let wal_path = temp.join("0000000000000001.wal");

        // Append torn write bytes to end of WAL
        {
            let mut f = fs.append_file(&wal_path).unwrap();
            f.write_all(b"torn_corrupted_tail_bytes").unwrap();
            f.sync_all().unwrap();
        }

        // Recover as last segment -> torn tail truncated cleanly
        let recovered = WalRecovery::recover_segment(fs.clone(), &wal_path, true).unwrap();
        assert_eq!(recovered.len(), 1);

        // Corrupt in the middle
        {
            let mut f = fs.open_file(&wal_path).unwrap();
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            data[20] ^= 0xFF; // corrupt batch seq or crc
            let mut f_write = fs.create_file(&wal_path).unwrap();
            f_write.write_all(&data).unwrap();
            f_write.sync_all().unwrap();
        }

        // Recover non-last segment -> corruption error
        let err = WalRecovery::recover_segment(fs.clone(), &wal_path, false);
        assert!(err.is_err());

        let _ = std::fs::remove_dir_all(&temp);
    }
}
