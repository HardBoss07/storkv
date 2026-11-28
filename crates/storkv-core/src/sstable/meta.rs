//! SSTable metadata block codec.
use super::footer::{BLOCK_HANDLE_SIZE, BlockHandle};
use crate::error::{Result, StorkvError};
use bytes::{Buf, BufMut, Bytes, BytesMut};

const META_BLOCK_SIZE: usize = 69;
const INDEX_ENTRY_COUNT_SIZE: usize = std::mem::size_of::<u32>();
const INDEX_KEY_LENGTH_SIZE: usize = std::mem::size_of::<u16>();
const INDEX_ENTRY_FIXED_SIZE: usize = INDEX_KEY_LENGTH_SIZE + BLOCK_HANDLE_SIZE;

/// Per-SSTable metadata block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetaBlock {
    /// Compaction level from L0 through L6.
    pub level: u8,
    /// Identifier of the owning scope.
    pub scope_id: u32,
    /// Number of entries in the file.
    pub entry_count: u64,
    /// Number of tombstone entries.
    pub tombstone_count: u64,
    /// Smallest sequence number in the file.
    pub min_seq: u64,
    /// Largest sequence number in the file.
    pub max_seq: u64,
    /// Smallest absolute expiry timestamp, or `0`.
    pub min_expires_at_ms: u64,
    /// Largest absolute expiry timestamp, or `0`.
    pub max_expires_at_ms: u64,
    /// Total user key bytes.
    pub raw_key_bytes: u64,
    /// Total value bytes.
    pub raw_val_bytes: u64,
}

const _: () = assert!(std::mem::size_of::<MetaBlock>() <= 72);
const _: () = assert!(std::mem::size_of::<BlockHandle>() == 16);

impl MetaBlock {
    /// Encode the metadata block in little-endian order.
    pub fn encode(&self) -> Vec<u8> {
        let mut buffer = BytesMut::with_capacity(META_BLOCK_SIZE);
        buffer.put_u32_le(self.scope_id);
        buffer.put_u8(self.level);
        buffer.put_u64_le(self.entry_count);
        buffer.put_u64_le(self.tombstone_count);
        buffer.put_u64_le(self.min_seq);
        buffer.put_u64_le(self.max_seq);
        buffer.put_u64_le(self.min_expires_at_ms);
        buffer.put_u64_le(self.max_expires_at_ms);
        buffer.put_u64_le(self.raw_key_bytes);
        buffer.put_u64_le(self.raw_val_bytes);
        buffer.to_vec()
    }

    /// Decode a metadata block.
    ///
    /// # Errors
    ///
    /// Returns `StorkvError::Corruption` when fewer than `META_BLOCK_SIZE`
    /// bytes are present.
    pub fn decode(mut data: &[u8]) -> Result<Self> {
        if data.remaining() < META_BLOCK_SIZE {
            return Err(StorkvError::Corruption("MetaBlock too short".into()));
        }
        Ok(Self {
            scope_id: data.get_u32_le(),
            level: data.get_u8(),
            entry_count: data.get_u64_le(),
            tombstone_count: data.get_u64_le(),
            min_seq: data.get_u64_le(),
            max_seq: data.get_u64_le(),
            min_expires_at_ms: data.get_u64_le(),
            max_expires_at_ms: data.get_u64_le(),
            raw_key_bytes: data.get_u64_le(),
            raw_val_bytes: data.get_u64_le(),
        })
    }
}

/// One index entry: the last key in a data block and the block handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    /// Last internal key in the data block.
    pub last_internal_key: Bytes,
    /// Offset and size of the data block.
    pub handle: BlockHandle,
}

/// Builder for the SSTable index block.
pub struct IndexBlockBuilder {
    entries: Vec<IndexEntry>,
}

impl IndexBlockBuilder {
    /// Create an empty builder.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Append one block boundary.
    pub fn add(&mut self, last_internal_key: Bytes, handle: BlockHandle) {
        self.entries.push(IndexEntry {
            last_internal_key,
            handle,
        });
    }

    /// Serialize the index block.
    pub fn finish(self) -> Vec<u8> {
        let mut buffer = BytesMut::new();
        buffer.put_u32_le(self.entries.len() as u32);
        for entry in self.entries {
            buffer.put_u16_le(entry.last_internal_key.len() as u16);
            buffer.extend_from_slice(&entry.last_internal_key);
            entry.handle.encode(&mut buffer);
        }
        buffer.to_vec()
    }
}

/// Decoded SSTable index block.
pub struct IndexBlockReader {
    /// Index entries in key order.
    pub entries: Vec<IndexEntry>,
}

impl IndexBlockReader {
    /// Decode an index block.
    ///
    /// # Errors
    ///
    /// Returns `StorkvError::Corruption` on a truncated or inconsistent block.
    pub fn decode(mut data: &[u8]) -> Result<Self> {
        if data.remaining() < INDEX_ENTRY_COUNT_SIZE {
            return Err(StorkvError::Corruption("Index block too short".into()));
        }
        let entry_count = data.get_u32_le() as usize;
        if entry_count > data.remaining() / INDEX_ENTRY_FIXED_SIZE {
            return Err(StorkvError::Corruption(
                "Index block entry count exceeds available bytes".into(),
            ));
        }

        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            if data.remaining() < INDEX_KEY_LENGTH_SIZE {
                return Err(StorkvError::Corruption("Truncated index key length".into()));
            }
            let key_length = data.get_u16_le() as usize;
            if data.remaining() < key_length + BLOCK_HANDLE_SIZE {
                return Err(StorkvError::Corruption("Truncated index entry".into()));
            }
            let mut key_bytes = vec![0u8; key_length];
            data.copy_to_slice(&mut key_bytes);
            let handle = BlockHandle::decode(&mut data)?;
            entries.push(IndexEntry {
                last_internal_key: Bytes::from(key_bytes),
                handle,
            });
        }
        Ok(Self { entries })
    }

    /// Find the first block whose last key is at or after the target key.
    pub fn find_block(&self, target_internal_key: &[u8]) -> Option<BlockHandle> {
        for entry in &self.entries {
            if entry.last_internal_key.as_ref() >= target_internal_key {
                return Some(entry.handle);
            }
        }
        self.entries.last().map(|entry| entry.handle)
    }
}
