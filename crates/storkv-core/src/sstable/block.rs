use crate::error::{Result, StorKvError};
use bytes::{BufMut, Bytes, BytesMut};

pub const RESTART_INTERVAL: usize = 16;
pub const TARGET_BLOCK_SIZE: usize = 4096;

pub struct DataBlockBuilder {
    buffer: BytesMut,
    restarts: Vec<u32>,
    pub(crate) last_key: Vec<u8>,
    entry_index: usize,
}

impl DataBlockBuilder {
    pub fn new() -> Self {
        Self {
            buffer: BytesMut::new(),
            restarts: vec![0],
            last_key: Vec::new(),
            entry_index: 0,
        }
    }

    pub fn add(&mut self, key: &[u8], value: &[u8]) {
        let shared = self.shared_prefix_length(key);
        let unshared = key.len() - shared;

        self.buffer.put_u16_le(shared as u16);
        self.buffer.put_u16_le(unshared as u16);
        self.buffer.put_u32_le(value.len() as u32);
        self.buffer.extend_from_slice(&key[shared..]);
        self.buffer.extend_from_slice(value);

        self.last_key.clear();
        self.last_key.extend_from_slice(key);
        self.entry_index += 1;
    }

    fn shared_prefix_length(&mut self, key: &[u8]) -> usize {
        if self.entry_index.is_multiple_of(RESTART_INTERVAL) {
            if self.entry_index > 0 {
                self.restarts.push(self.buffer.len() as u32);
            }
            return 0;
        }

        let mut shared = 0;
        while shared < key.len()
            && shared < self.last_key.len()
            && key[shared] == self.last_key[shared]
        {
            shared += 1;
        }
        shared
    }

    pub fn estimated_size(&self) -> usize {
        self.buffer.len() + (self.restarts.len() * 4) + 4
    }

    pub fn finish(mut self) -> Vec<u8> {
        for &restart_offset in &self.restarts {
            self.buffer.put_u32_le(restart_offset);
        }
        self.buffer.put_u32_le(self.restarts.len() as u32);
        self.buffer.to_vec()
    }
}

pub struct DataBlockReader {
    data: Bytes,
    restarts: Vec<u32>,
}

impl DataBlockReader {
    pub fn new(data: Bytes) -> Result<Self> {
        if data.len() < 4 {
            return Err(StorKvError::Corruption("Data block too short".into()));
        }
        let restart_count = u32::from_le_bytes(
            data[data.len() - 4..]
                .try_into()
                .map_err(|_| StorKvError::Corruption("Invalid restart count field".into()))?,
        ) as usize;
        let restart_array_length = restart_count.checked_mul(4).ok_or_else(|| {
            StorKvError::Corruption("Restart array length overflow in data block".into())
        })?;
        if data.len() < 4 + restart_array_length {
            return Err(StorKvError::Corruption(
                "Corrupted restart array in data block".into(),
            ));
        }
        if restart_count == 0 {
            return Err(StorKvError::Corruption(
                "Data block declares zero restart points".into(),
            ));
        }

        let restart_offset_start = data.len() - 4 - restart_array_length;
        let mut restarts = Vec::with_capacity(restart_count);
        for index in 0..restart_count {
            let position = restart_offset_start + index * 4;
            let offset =
                u32::from_le_bytes(data[position..position + 4].try_into().map_err(|_| {
                    StorKvError::Corruption("Invalid restart offset field in data block".into())
                })?);
            if offset as usize > restart_offset_start {
                return Err(StorKvError::Corruption(
                    "Restart offset points outside the data section".into(),
                ));
            }
            restarts.push(offset);
        }

        Ok(Self { data, restarts })
    }

    pub fn entries(&self) -> Result<Vec<(Bytes, Bytes)>> {
        let entry_start = *self.restarts.first().ok_or_else(|| {
            StorKvError::Corruption("Data block contains no restart points".into())
        })? as usize;
        let restart_bytes = self.restarts.len().checked_mul(4).ok_or_else(|| {
            StorKvError::Corruption("Restart array length overflow in data block".into())
        })?;
        let entry_end = (self.data.len())
            .checked_sub(4 + restart_bytes)
            .ok_or_else(|| StorKvError::Corruption("Corrupted data block layout".into()))?;
        if entry_start > entry_end {
            return Err(StorKvError::Corruption(
                "Restart section starts beyond the entry region".into(),
            ));
        }

        let mut entries = Vec::new();
        let mut current_key = Vec::new();
        let mut offset = entry_start;

        while offset < entry_end {
            let (key, value, next_offset) =
                self.decode_entry(offset, entry_end, &mut current_key)?;
            offset = next_offset;
            entries.push((key, value));
        }
        Ok(entries)
    }

    fn decode_entry(
        &self,
        offset: usize,
        entry_end: usize,
        current_key: &mut Vec<u8>,
    ) -> Result<(Bytes, Bytes, usize)> {
        let header_end = offset
            .checked_add(8)
            .ok_or_else(|| StorKvError::Corruption("Entry offset overflow".into()))?;
        if header_end > entry_end {
            return Err(StorKvError::Corruption("Truncated entry header".into()));
        }

        let shared = u16::from_le_bytes(
            self.data[offset..offset + 2]
                .try_into()
                .map_err(|_| StorKvError::Corruption("Invalid shared prefix length".into()))?,
        ) as usize;
        let unshared = u16::from_le_bytes(
            self.data[offset + 2..offset + 4]
                .try_into()
                .map_err(|_| StorKvError::Corruption("Invalid unshared length".into()))?,
        ) as usize;
        let value_length = u32::from_le_bytes(
            self.data[offset + 4..offset + 8]
                .try_into()
                .map_err(|_| StorKvError::Corruption("Invalid value length".into()))?,
        ) as usize;

        let payload_end = header_end
            .checked_add(unshared)
            .and_then(|end| end.checked_add(value_length))
            .ok_or_else(|| StorKvError::Corruption("Entry payload length overflow".into()))?;
        if payload_end > entry_end {
            return Err(StorKvError::Corruption("Truncated entry key/val".into()));
        }
        if shared > current_key.len() {
            return Err(StorKvError::Corruption(
                "Shared prefix exceeds previous key length".into(),
            ));
        }

        current_key.truncate(shared);
        current_key.extend_from_slice(&self.data[header_end..header_end + unshared]);

        let value_start = header_end + unshared;
        let value = self.data.slice(value_start..value_start + value_length);
        Ok((Bytes::copy_from_slice(current_key), value, payload_end))
    }

    pub fn seek_ge(&self, target_internal_key: &[u8]) -> Result<Option<(Bytes, Bytes)>> {
        for (key, value) in self.entries()? {
            if key.as_ref() >= target_internal_key {
                return Ok(Some((key, value)));
            }
        }
        Ok(None)
    }
}
