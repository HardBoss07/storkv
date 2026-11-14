use super::footer::BlockHandle;
use crate::error::{Result, StorKvError};
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetaBlock {
    pub entry_count: u64,
    pub tombstone_count: u64,
    pub min_seq: u64,
    pub max_seq: u64,
    pub min_expires_at_ms: u64,
    pub max_expires_at_ms: u64,
    pub raw_key_bytes: u64,
    pub raw_val_bytes: u64,
    pub scope_id: u32,
    pub level: u8,
}

const _: () = assert!(std::mem::size_of::<MetaBlock>() <= 72);
const _: () = assert!(std::mem::size_of::<BlockHandle>() == 16);

impl MetaBlock {
    pub fn encode(&self) -> Vec<u8> {
        let mut buffer = BytesMut::with_capacity(69);
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

    pub fn decode(mut data: &[u8]) -> Result<Self> {
        if data.remaining() < 69 {
            return Err(StorKvError::Corruption("MetaBlock too short".into()));
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub last_internal_key: Bytes,
    pub handle: BlockHandle,
}

pub struct IndexBlockBuilder {
    entries: Vec<IndexEntry>,
}

impl IndexBlockBuilder {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn add(&mut self, last_internal_key: Bytes, handle: BlockHandle) {
        self.entries.push(IndexEntry {
            last_internal_key,
            handle,
        });
    }

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

pub struct IndexBlockReader {
    pub entries: Vec<IndexEntry>,
}

impl IndexBlockReader {
    pub fn decode(mut data: &[u8]) -> Result<Self> {
        if data.remaining() < 4 {
            return Err(StorKvError::Corruption("Index block too short".into()));
        }
        let entry_count = data.get_u32_le() as usize;
        if entry_count > data.remaining() / 18 {
            return Err(StorKvError::Corruption(
                "Index block entry count exceeds available bytes".into(),
            ));
        }

        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            if data.remaining() < 2 {
                return Err(StorKvError::Corruption("Truncated index key length".into()));
            }
            let key_length = data.get_u16_le() as usize;
            if data.remaining() < key_length + 16 {
                return Err(StorKvError::Corruption("Truncated index entry".into()));
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

    pub fn find_block(&self, target_internal_key: &[u8]) -> Option<BlockHandle> {
        for entry in &self.entries {
            if entry.last_internal_key.as_ref() >= target_internal_key {
                return Some(entry.handle);
            }
        }
        self.entries.last().map(|entry| entry.handle)
    }
}
