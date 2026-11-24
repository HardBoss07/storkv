use crate::db::DbInner;
use crate::format::{InternalKey, ValueType};
use crate::scope::{Scope, StorageMode};
use crate::types::{SequenceNumber, TimestampMs};
use crate::vfs::FileSystem;
use bytes::{BufMut, BytesMut};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub(crate) const SNAPSHOT_MAGIC: &[u8; 9] = b"STORKSNAP";
pub(crate) const SNAPSHOT_VERSION: u16 = 1;
const SNAPSHOT_DIR: &str = "snapshots";
/// Size of the fixed-width fields that follow the magic bytes: version,
/// sequence number, and entry count.
const SNAPSHOT_FIXED_HEADER_SIZE: usize = 2 + 8 + 8;
/// Size of a little-endian `u16` field.
const U16_SIZE: usize = 2;
/// Size of a little-endian `u32` field.
const U32_SIZE: usize = 4;
/// Size of a little-endian `u64` field.
const U64_SIZE: usize = 8;

/// Path of the recovery snapshot for one scope.
pub(crate) fn snapshot_path(db_root: &Path, scope_id: u32) -> PathBuf {
    db_root
        .join(SNAPSHOT_DIR)
        .join(format!("{:08x}.snapshot", scope_id))
}

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Flush every scope and write recovery snapshots for memory scopes.
    ///
    /// # Errors
    ///
    /// Returns an error when a flush or a snapshot write fails.
    pub fn flush_all(&self) -> crate::Result<()> {
        let scopes: Vec<_> = self.scopes.read().values().cloned().collect();
        for scope in &scopes {
            self.flush_scope_memtable(scope)?;
        }
        self.write_memory_snapshots(&scopes)?;
        self.wal_mgr.sync_active()
    }

    fn write_memory_snapshots(&self, scopes: &[Arc<Scope<FS>>]) -> crate::Result<()> {
        let snapshot_dir = self.db_root.join(SNAPSHOT_DIR);
        self.fs.create_dir_all(&snapshot_dir)?;
        for scope in scopes {
            if scope.options.storage_mode != StorageMode::InMemoryPersistent {
                continue;
            }
            self.write_scope_snapshot(scope)?;
        }
        self.fs.sync_dir(&snapshot_dir)
    }

    fn write_scope_snapshot(&self, scope: &Arc<Scope<FS>>) -> crate::Result<()> {
        let read_seq = self.last_visible_seq.load(Ordering::Acquire);
        let entries = self.scan(scope.scope_id, None, None, read_seq)?;
        let now_ms = TimestampMs::now();

        let mut body = BytesMut::new();
        body.put_slice(SNAPSHOT_MAGIC);
        body.put_u16_le(SNAPSHOT_VERSION);
        body.put_u64_le(read_seq);
        body.put_u64_le(entries.len() as u64);
        for (key, value) in entries {
            let expires_at_ms = scope
                .key_meta(key.as_ref(), SequenceNumber::new(read_seq), now_ms)?
                .map(|meta| meta.expires_at_ms)
                .unwrap_or(0);
            body.put_u32_le(key.len() as u32);
            body.put_slice(&key);
            body.put_u32_le(value.len() as u32);
            body.put_slice(&value);
            body.put_u64_le(expires_at_ms);
        }

        let path = snapshot_path(&self.db_root, scope.scope_id);
        let mut file = self.fs.create_file(&path)?;
        file.write_all(&body)?;
        file.sync_all()
    }
}

/// Load the recovery snapshot of one memory scope.
///
/// # Errors
///
/// Returns an error when the snapshot is present but malformed.
pub(crate) fn load_scope_snapshot<FS: FileSystem>(
    fs: &FS,
    db_root: &Path,
    scope: &Arc<Scope<FS>>,
) -> crate::Result<u64> {
    let path = snapshot_path(db_root, scope.scope_id);
    if !fs.exists(&path) {
        return Ok(0);
    }
    let mut file = fs.open_file(&path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let entries = parse_snapshot(&bytes)?;

    for entry in &entries.entries {
        let internal_key = InternalKey::new(
            entry.key.clone(),
            entries.seq,
            ValueType::Value,
            entry.expires_at_ms,
        );
        scope.active_memtable.read().insert(
            internal_key.encode(),
            entry.value.clone(),
            entries.seq,
        );
    }
    scope.snapshot_seq.store(entries.seq, Ordering::SeqCst);
    Ok(entries.seq)
}

struct SnapshotEntry {
    key: bytes::Bytes,
    value: bytes::Bytes,
    expires_at_ms: u64,
}

struct LoadedSnapshot {
    seq: u64,
    entries: Vec<SnapshotEntry>,
}

fn parse_snapshot(bytes: &[u8]) -> crate::Result<LoadedSnapshot> {
    let header_len = SNAPSHOT_MAGIC.len() + SNAPSHOT_FIXED_HEADER_SIZE;
    if bytes.len() < header_len || &bytes[..SNAPSHOT_MAGIC.len()] != SNAPSHOT_MAGIC {
        return Err(crate::StorkvError::Corruption(
            "Invalid recovery snapshot header".into(),
        ));
    }
    let mut cursor = SNAPSHOT_MAGIC.len();
    let version = read_u16(bytes, &mut cursor)?;
    if version != SNAPSHOT_VERSION {
        return Err(crate::StorkvError::Corruption(
            "Unsupported recovery snapshot version".into(),
        ));
    }
    let seq = read_u64(bytes, &mut cursor)?;
    let count = read_u64(bytes, &mut cursor)?;
    let mut entries = Vec::new();
    for _ in 0..count {
        let key_len = read_u32(bytes, &mut cursor)? as usize;
        let key = read_bytes(bytes, &mut cursor, key_len)?;
        let value_len = read_u32(bytes, &mut cursor)? as usize;
        let value = read_bytes(bytes, &mut cursor, value_len)?;
        let expires_at_ms = read_u64(bytes, &mut cursor)?;
        entries.push(SnapshotEntry {
            key,
            value,
            expires_at_ms,
        });
    }
    Ok(LoadedSnapshot { seq, entries })
}

fn read_u16(bytes: &[u8], cursor: &mut usize) -> crate::Result<u16> {
    let slice = take(bytes, cursor, U16_SIZE)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> crate::Result<u32> {
    let slice = take(bytes, cursor, U32_SIZE)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64(bytes: &[u8], cursor: &mut usize) -> crate::Result<u64> {
    let slice = take(bytes, cursor, U64_SIZE)?;
    let mut raw = [0u8; U64_SIZE];
    raw.copy_from_slice(slice);
    Ok(u64::from_le_bytes(raw))
}

fn read_bytes(bytes: &[u8], cursor: &mut usize, length: usize) -> crate::Result<bytes::Bytes> {
    Ok(bytes::Bytes::copy_from_slice(take(bytes, cursor, length)?))
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, length: usize) -> crate::Result<&'a [u8]> {
    let end = cursor
        .checked_add(length)
        .ok_or_else(|| crate::StorkvError::Corruption("Snapshot length overflow".into()))?;
    if end > bytes.len() {
        return Err(crate::StorkvError::Corruption(
            "Truncated recovery snapshot".into(),
        ));
    }
    let slice = &bytes[*cursor..end];
    *cursor = end;
    Ok(slice)
}
