use crate::error::{Result, StorKvError};
use crate::format::compute_crc32c;
use crate::vfs::{File, FileSystem};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

pub const MANIFEST_MAGIC: &[u8; 8] = b"STORKMNF";
pub const MANIFEST_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurabilityMode {
    StrictFsync = 0,
    Buffered = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetentionMode {
    Permanent = 0,
    Ttl = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionStyleMode {
    Leveled = 0,
    Fifo = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeMeta {
    pub scope_id: u32,
    pub durability: DurabilityMode,
    pub sync_ms: u32,
    pub retention: RetentionMode,
    pub default_ttl_ms: u64,
    pub compaction_style: CompactionStyleMode,
    pub name: String,
    pub dropped: bool,
    pub drop_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstFileMeta {
    pub scope_id: u32,
    pub level: u8,
    pub file_num: u64,
    pub file_size: u64,
    pub min_seq: u64,
    pub max_seq: u64,
    pub max_expires_at_ms: u64,
    pub min_key: Bytes,
    pub max_key: Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMeta {
    pub name: String,
    pub seq: u64,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditRecord {
    SetNextSequence(u64),
    SetWalNumber(u64),
    CreateScope {
        scope_id: u32,
        durability: u8,
        sync_ms: u32,
        retention: u8,
        default_ttl_ms: u64,
        compaction_style: u8,
        name: String,
    },
    DropScope {
        scope_id: u32,
        drop_seq: u64,
    },
    AddSstFile(SstFileMeta),
    DeleteSstFile {
        scope_id: u32,
        level: u8,
        file_num: u64,
    },
    PinSnapshot {
        seq: u64,
        created_at_ms: u64,
        name: String,
    },
    UnpinSnapshot {
        name: String,
    },
}

impl EditRecord {
    pub fn encode(&self, buf: &mut BytesMut) {
        match self {
            EditRecord::SetNextSequence(seq) => {
                buf.put_u8(0x01);
                buf.put_u64_le(*seq);
            }
            EditRecord::SetWalNumber(wal_num) => {
                buf.put_u8(0x02);
                buf.put_u64_le(*wal_num);
            }
            EditRecord::CreateScope {
                scope_id,
                durability,
                sync_ms,
                retention,
                default_ttl_ms,
                compaction_style,
                name,
            } => {
                buf.put_u8(0x03);
                buf.put_u32_le(*scope_id);
                buf.put_u8(*durability);
                buf.put_u32_le(*sync_ms);
                buf.put_u8(*retention);
                buf.put_u64_le(*default_ttl_ms);
                buf.put_u8(*compaction_style);
                buf.put_u16_le(name.len() as u16);
                buf.extend_from_slice(name.as_bytes());
            }
            EditRecord::DropScope { scope_id, drop_seq } => {
                buf.put_u8(0x04);
                buf.put_u32_le(*scope_id);
                buf.put_u64_le(*drop_seq);
            }
            EditRecord::AddSstFile(meta) => {
                buf.put_u8(0x05);
                buf.put_u32_le(meta.scope_id);
                buf.put_u8(meta.level);
                buf.put_u64_le(meta.file_num);
                buf.put_u64_le(meta.file_size);
                buf.put_u64_le(meta.min_seq);
                buf.put_u64_le(meta.max_seq);
                buf.put_u64_le(meta.max_expires_at_ms);
                buf.put_u16_le(meta.min_key.len() as u16);
                buf.extend_from_slice(&meta.min_key);
                buf.put_u16_le(meta.max_key.len() as u16);
                buf.extend_from_slice(&meta.max_key);
            }
            EditRecord::DeleteSstFile {
                scope_id,
                level,
                file_num,
            } => {
                buf.put_u8(0x06);
                buf.put_u32_le(*scope_id);
                buf.put_u8(*level);
                buf.put_u64_le(*file_num);
            }
            EditRecord::PinSnapshot {
                seq,
                created_at_ms,
                name,
            } => {
                buf.put_u8(0x07);
                buf.put_u64_le(*seq);
                buf.put_u64_le(*created_at_ms);
                buf.put_u16_le(name.len() as u16);
                buf.extend_from_slice(name.as_bytes());
            }
            EditRecord::UnpinSnapshot { name } => {
                buf.put_u8(0x08);
                buf.put_u16_le(name.len() as u16);
                buf.extend_from_slice(name.as_bytes());
            }
        }
    }

    pub fn decode(buf: &mut impl Buf) -> Result<Self> {
        if buf.remaining() < 1 {
            return Err(StorKvError::Corruption("Incomplete EditRecord tag".into()));
        }
        let tag = buf.get_u8();
        match tag {
            0x01 => {
                if buf.remaining() < 8 {
                    return Err(StorKvError::Corruption("Incomplete SetNextSequence".into()));
                }
                Ok(EditRecord::SetNextSequence(buf.get_u64_le()))
            }
            0x02 => {
                if buf.remaining() < 8 {
                    return Err(StorKvError::Corruption("Incomplete SetWalNumber".into()));
                }
                Ok(EditRecord::SetWalNumber(buf.get_u64_le()))
            }
            0x03 => {
                if buf.remaining() < 4 + 1 + 4 + 1 + 8 + 1 + 2 {
                    return Err(StorKvError::Corruption("Incomplete CreateScope".into()));
                }
                let scope_id = buf.get_u32_le();
                let durability = buf.get_u8();
                let sync_ms = buf.get_u32_le();
                let retention = buf.get_u8();
                let default_ttl_ms = buf.get_u64_le();
                let compaction_style = buf.get_u8();
                let name_len = buf.get_u16_le() as usize;
                if buf.remaining() < name_len {
                    return Err(StorKvError::Corruption(
                        "Incomplete CreateScope name".into(),
                    ));
                }
                let mut name_bytes = vec![0u8; name_len];
                buf.copy_to_slice(&mut name_bytes);
                let name = String::from_utf8(name_bytes)
                    .map_err(|e| StorKvError::Corruption(e.to_string()))?;
                Ok(EditRecord::CreateScope {
                    scope_id,
                    durability,
                    sync_ms,
                    retention,
                    default_ttl_ms,
                    compaction_style,
                    name,
                })
            }
            0x04 => {
                if buf.remaining() < 4 + 8 {
                    return Err(StorKvError::Corruption("Incomplete DropScope".into()));
                }
                let scope_id = buf.get_u32_le();
                let drop_seq = buf.get_u64_le();
                Ok(EditRecord::DropScope { scope_id, drop_seq })
            }
            0x05 => {
                if buf.remaining() < 4 + 1 + 8 + 8 + 8 + 8 + 8 + 2 {
                    return Err(StorKvError::Corruption("Incomplete AddSstFile".into()));
                }
                let scope_id = buf.get_u32_le();
                let level = buf.get_u8();
                let file_num = buf.get_u64_le();
                let file_size = buf.get_u64_le();
                let min_seq = buf.get_u64_le();
                let max_seq = buf.get_u64_le();
                let max_expires_at_ms = buf.get_u64_le();
                let min_key_len = buf.get_u16_le() as usize;
                if buf.remaining() < min_key_len + 2 {
                    return Err(StorKvError::Corruption(
                        "Incomplete AddSstFile min_key".into(),
                    ));
                }
                let mut min_key = vec![0u8; min_key_len];
                buf.copy_to_slice(&mut min_key);
                let max_key_len = buf.get_u16_le() as usize;
                if buf.remaining() < max_key_len {
                    return Err(StorKvError::Corruption(
                        "Incomplete AddSstFile max_key".into(),
                    ));
                }
                let mut max_key = vec![0u8; max_key_len];
                buf.copy_to_slice(&mut max_key);

                Ok(EditRecord::AddSstFile(SstFileMeta {
                    scope_id,
                    level,
                    file_num,
                    file_size,
                    min_seq,
                    max_seq,
                    max_expires_at_ms,
                    min_key: Bytes::from(min_key),
                    max_key: Bytes::from(max_key),
                }))
            }
            0x06 => {
                if buf.remaining() < 4 + 1 + 8 {
                    return Err(StorKvError::Corruption("Incomplete DeleteSstFile".into()));
                }
                let scope_id = buf.get_u32_le();
                let level = buf.get_u8();
                let file_num = buf.get_u64_le();
                Ok(EditRecord::DeleteSstFile {
                    scope_id,
                    level,
                    file_num,
                })
            }
            0x07 => {
                if buf.remaining() < 8 + 8 + 2 {
                    return Err(StorKvError::Corruption("Incomplete PinSnapshot".into()));
                }
                let seq = buf.get_u64_le();
                let created_at_ms = buf.get_u64_le();
                let name_len = buf.get_u16_le() as usize;
                if buf.remaining() < name_len {
                    return Err(StorKvError::Corruption(
                        "Incomplete PinSnapshot name".into(),
                    ));
                }
                let mut name_bytes = vec![0u8; name_len];
                buf.copy_to_slice(&mut name_bytes);
                let name = String::from_utf8(name_bytes)
                    .map_err(|e| StorKvError::Corruption(e.to_string()))?;
                Ok(EditRecord::PinSnapshot {
                    seq,
                    created_at_ms,
                    name,
                })
            }
            0x08 => {
                if buf.remaining() < 2 {
                    return Err(StorKvError::Corruption("Incomplete UnpinSnapshot".into()));
                }
                let name_len = buf.get_u16_le() as usize;
                if buf.remaining() < name_len {
                    return Err(StorKvError::Corruption(
                        "Incomplete UnpinSnapshot name".into(),
                    ));
                }
                let mut name_bytes = vec![0u8; name_len];
                buf.copy_to_slice(&mut name_bytes);
                let name = String::from_utf8(name_bytes)
                    .map_err(|e| StorKvError::Corruption(e.to_string()))?;
                Ok(EditRecord::UnpinSnapshot { name })
            }
            _ => Err(StorKvError::Corruption(format!(
                "Unknown EditRecord tag: 0x{:02x}",
                tag
            ))),
        }
    }
}

/// A VersionEdit frame holding multiple EditRecords.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionEdit {
    pub edits: Vec<EditRecord>,
}

impl VersionEdit {
    pub fn new() -> Self {
        Self { edits: Vec::new() }
    }

    pub fn add(&mut self, edit: EditRecord) {
        self.edits.push(edit);
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut body = BytesMut::new();
        body.put_u16_le(self.edits.len() as u16);
        for edit in &self.edits {
            edit.encode(&mut body);
        }

        let payload_len = body.len() as u32;
        let mut frame_data = BytesMut::with_capacity(4 + body.len());
        frame_data.put_u32_le(payload_len);
        frame_data.extend_from_slice(&body);

        let crc = compute_crc32c(&frame_data);

        let mut out = Vec::with_capacity(4 + frame_data.len());
        out.put_u32_le(crc);
        out.extend_from_slice(&frame_data);
        out
    }

    pub fn decode(buf: &[u8]) -> Result<(Self, usize)> {
        if buf.len() < 8 {
            return Err(StorKvError::Corruption(
                "Truncated VersionEdit frame header".into(),
            ));
        }
        let crc = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        let payload_len = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
        let total_frame_len = 8 + payload_len;

        if buf.len() < total_frame_len {
            return Err(StorKvError::Corruption(
                "Truncated VersionEdit frame payload".into(),
            ));
        }

        let calculated_crc = compute_crc32c(&buf[4..total_frame_len]);
        if crc != calculated_crc {
            return Err(StorKvError::Corruption(format!(
                "VersionEdit CRC mismatch: expected 0x{:08x}, computed 0x{:08x}",
                crc, calculated_crc
            )));
        }

        let mut payload = &buf[8..total_frame_len];
        if payload.remaining() < 2 {
            return Err(StorKvError::Corruption("Incomplete edit_count".into()));
        }
        let edit_count = payload.get_u16_le() as usize;
        let mut edits = Vec::with_capacity(edit_count);
        for _ in 0..edit_count {
            edits.push(EditRecord::decode(&mut payload)?);
        }

        Ok((VersionEdit { edits }, total_frame_len))
    }
}

/// In-memory engine state reconstructed from MANIFEST
#[derive(Debug, Clone, Default)]
pub struct ManifestState {
    pub next_seq: u64,
    pub min_unflushed_wal_num: u64,
    pub scopes: HashMap<u32, ScopeMeta>,
    pub scope_by_name: HashMap<String, u32>,
    pub sst_files: BTreeMap<(u32, u8, u64), SstFileMeta>,
    pub pinned_snapshots: HashMap<String, SnapshotMeta>,
}

impl ManifestState {
    pub fn apply(&mut self, edit: &VersionEdit) {
        for record in &edit.edits {
            match record {
                EditRecord::SetNextSequence(seq) => {
                    if *seq > self.next_seq {
                        self.next_seq = *seq;
                    }
                }
                EditRecord::SetWalNumber(wal_num) => {
                    self.min_unflushed_wal_num = *wal_num;
                }
                EditRecord::CreateScope {
                    scope_id,
                    durability,
                    sync_ms,
                    retention,
                    default_ttl_ms,
                    compaction_style,
                    name,
                } => {
                    let d = match durability {
                        0 => DurabilityMode::StrictFsync,
                        _ => DurabilityMode::Buffered,
                    };
                    let r = match retention {
                        0 => RetentionMode::Permanent,
                        _ => RetentionMode::Ttl,
                    };
                    let c = match compaction_style {
                        0 => CompactionStyleMode::Leveled,
                        _ => CompactionStyleMode::Fifo,
                    };
                    let meta = ScopeMeta {
                        scope_id: *scope_id,
                        durability: d,
                        sync_ms: *sync_ms,
                        retention: r,
                        default_ttl_ms: *default_ttl_ms,
                        compaction_style: c,
                        name: name.clone(),
                        dropped: false,
                        drop_seq: 0,
                    };
                    self.scopes.insert(*scope_id, meta);
                    self.scope_by_name.insert(name.clone(), *scope_id);
                }
                EditRecord::DropScope { scope_id, drop_seq } => {
                    if let Some(scope) = self.scopes.get_mut(scope_id) {
                        scope.dropped = true;
                        scope.drop_seq = *drop_seq;
                        self.scope_by_name.remove(&scope.name);
                    }
                }
                EditRecord::AddSstFile(meta) => {
                    self.sst_files
                        .insert((meta.scope_id, meta.level, meta.file_num), meta.clone());
                }
                EditRecord::DeleteSstFile {
                    scope_id,
                    level,
                    file_num,
                } => {
                    self.sst_files.remove(&(*scope_id, *level, *file_num));
                }
                EditRecord::PinSnapshot {
                    seq,
                    created_at_ms,
                    name,
                } => {
                    self.pinned_snapshots.insert(
                        name.clone(),
                        SnapshotMeta {
                            name: name.clone(),
                            seq: *seq,
                            created_at_ms: *created_at_ms,
                        },
                    );
                }
                EditRecord::UnpinSnapshot { name } => {
                    self.pinned_snapshots.remove(name);
                }
            }
        }
    }
}

pub struct ManifestManager<FS: FileSystem> {
    pub fs: Arc<FS>,
    pub db_root: PathBuf,
    pub active_manifest_num: AtomicU64,
    state: parking_lot::RwLock<ManifestState>,
    writer: parking_lot::Mutex<Box<dyn File>>,
}

impl<FS: FileSystem> ManifestManager<FS> {
    pub fn open(fs: Arc<FS>, db_root: &Path) -> Result<Self> {
        let current_path = db_root.join("CURRENT");
        let mut state = ManifestState::default();
        let (active_num, file) = if fs.exists(&current_path) {
            let mut cur_file = fs.open_file(&current_path)?;
            let mut bytes = Vec::new();
            cur_file.read_to_end(&mut bytes)?;
            let contents =
                String::from_utf8(bytes).map_err(|e| StorKvError::Corruption(e.to_string()))?;
            let manifest_filename = contents.trim();
            if !manifest_filename.starts_with("MANIFEST-") {
                return Err(StorKvError::Corruption(
                    "Invalid CURRENT file content".into(),
                ));
            }
            let num_str = &manifest_filename["MANIFEST-".len()..];
            let num = u64::from_str_radix(num_str, 16).map_err(|_| {
                StorKvError::Corruption("Invalid manifest file number in CURRENT".into())
            })?;

            let manifest_path = db_root.join(manifest_filename);
            let mut mfile = fs.open_file(&manifest_path)?;
            let mut data = Vec::new();
            mfile.read_to_end(&mut data)?;

            if data.len() < 16 {
                return Err(StorKvError::Corruption("Manifest header too short".into()));
            }
            if &data[0..8] != MANIFEST_MAGIC {
                return Err(StorKvError::Corruption("Invalid Manifest magic".into()));
            }

            let mut offset = 16;
            while offset < data.len() {
                match VersionEdit::decode(&data[offset..]) {
                    Ok((edit, consumed)) => {
                        state.apply(&edit);
                        offset += consumed;
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }

            let app_file = fs.append_file(&manifest_path)?;
            (num, app_file)
        } else {
            // New database: initialize MANIFEST-0000000000000001
            let num = 1;
            let manifest_filename = format!("MANIFEST-{:016x}", num);
            let manifest_path = db_root.join(&manifest_filename);
            let mut mfile = fs.create_file(&manifest_path)?;

            let mut header = BytesMut::with_capacity(16);
            header.extend_from_slice(MANIFEST_MAGIC);
            header.put_u16_le(MANIFEST_VERSION);
            header.put_u16_le(0); // reserved
            let now_sec = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as u32;
            header.put_u32_le(now_sec);
            mfile.write_all(&header)?;
            mfile.sync_all()?;

            // Atomic CURRENT activation
            let tmp_current = db_root.join("CURRENT.tmp");
            {
                let mut tmp_file = fs.create_file(&tmp_current)?;
                tmp_file.write_all(format!("{}\n", manifest_filename).as_bytes())?;
                tmp_file.sync_all()?;
            }
            fs.rename(&tmp_current, &current_path)?;
            fs.sync_dir(db_root)?;

            let app_file = fs.append_file(&manifest_path)?;
            (num, app_file)
        };

        Ok(Self {
            fs,
            db_root: db_root.to_path_buf(),
            active_manifest_num: AtomicU64::new(active_num),
            state: parking_lot::RwLock::new(state),
            writer: parking_lot::Mutex::new(file),
        })
    }

    pub fn log_edit(&self, edit: VersionEdit) -> Result<()> {
        let bytes = edit.encode();
        let mut writer = self.writer.lock();
        writer.write_all(&bytes)?;
        writer.sync_all()?;
        let mut state = self.state.write();
        state.apply(&edit);
        Ok(())
    }

    pub fn state(&self) -> parking_lot::RwLockReadGuard<'_, ManifestState> {
        self.state.read()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::PosixFs;

    #[test]
    fn test_manifest_roundtrip_all_edits() {
        let temp = std::env::temp_dir().join(format!("storkv_test_mnf_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let fs = Arc::new(PosixFs);
        fs.create_dir_all(&temp).unwrap();

        let mgr = ManifestManager::open(fs.clone(), &temp).unwrap();

        let mut edit = VersionEdit::new();
        edit.add(EditRecord::SetNextSequence(100));
        edit.add(EditRecord::SetWalNumber(2));
        edit.add(EditRecord::CreateScope {
            scope_id: 1,
            durability: 0,
            sync_ms: 100,
            retention: 1,
            default_ttl_ms: 60000,
            compaction_style: 1,
            name: "sessions".into(),
        });
        edit.add(EditRecord::AddSstFile(SstFileMeta {
            scope_id: 1,
            level: 0,
            file_num: 5,
            file_size: 4096,
            min_seq: 1,
            max_seq: 50,
            max_expires_at_ms: 9999999,
            min_key: Bytes::from_static(b"a"),
            max_key: Bytes::from_static(b"z"),
        }));
        edit.add(EditRecord::PinSnapshot {
            seq: 50,
            created_at_ms: 1234567,
            name: "snap-1".into(),
        });

        mgr.log_edit(edit).unwrap();

        drop(mgr);

        // Reopen manifest
        let mgr2 = ManifestManager::open(fs.clone(), &temp).unwrap();
        let state = mgr2.state();
        assert_eq!(state.next_seq, 100);
        assert_eq!(state.min_unflushed_wal_num, 2);
        assert!(state.scopes.contains_key(&1));
        assert_eq!(state.scopes.get(&1).unwrap().name, "sessions");
        assert!(state.sst_files.contains_key(&(1, 0, 5)));
        assert!(state.pinned_snapshots.contains_key("snap-1"));

        let _ = std::fs::remove_dir_all(&temp);
    }
}
