use crate::error::{Result, StorKvError};
use crate::format::compute_crc32c;
use bytes::{Buf, BufMut};

pub const SST_MAGIC: &[u8; 8] = b"STORKSST";
pub const FOOTER_SIZE: usize = 56;
pub const BLOCK_TRAILER_SIZE: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockHandle {
    pub offset: u64,
    pub size: u64,
}

impl BlockHandle {
    pub fn encode(&self, buffer: &mut impl BufMut) {
        buffer.put_u64_le(self.offset);
        buffer.put_u64_le(self.size);
    }

    pub fn decode(buffer: &mut impl Buf) -> Result<Self> {
        if buffer.remaining() < 16 {
            return Err(StorKvError::Corruption("Incomplete BlockHandle".into()));
        }
        Ok(Self {
            offset: buffer.get_u64_le(),
            size: buffer.get_u64_le(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompressionType {
    None = 0x00,
    Lz4 = 0x01,
    Zstd = 0x02,
}

impl CompressionType {
    pub fn from_u8(value: u8) -> Result<Self> {
        match value {
            0x00 => Ok(CompressionType::None),
            0x01 => Ok(CompressionType::Lz4),
            0x02 => Ok(CompressionType::Zstd),
            _ => Err(StorKvError::Corruption(format!(
                "Unknown compression type: {}",
                value
            ))),
        }
    }
}

pub fn compress_and_frame_block(payload: &[u8], compression: CompressionType) -> Result<Vec<u8>> {
    let mut stored_payload = compress_payload(payload, compression)?;

    let mut checksum_input = Vec::with_capacity(stored_payload.len() + 1);
    checksum_input.extend_from_slice(&stored_payload);
    checksum_input.push(compression as u8);
    let checksum = compute_crc32c(&checksum_input);

    stored_payload.push(compression as u8);
    stored_payload.extend_from_slice(&checksum.to_le_bytes());
    Ok(stored_payload)
}

fn compress_payload(payload: &[u8], compression: CompressionType) -> Result<Vec<u8>> {
    match compression {
        CompressionType::None => Ok(payload.to_vec()),
        CompressionType::Lz4 => Ok(lz4_flex::compress_prepend_size(payload)),
        CompressionType::Zstd => {
            let mut compressed = Vec::new();
            compressed.put_u32_le(payload.len() as u32);
            let encoded = zstd::stream::encode_all(payload, 0)
                .map_err(|error| StorKvError::Corruption(error.to_string()))?;
            compressed.extend_from_slice(&encoded);
            Ok(compressed)
        }
    }
}

pub fn decompress_and_verify_block(data_with_trailer: &[u8]) -> Result<Vec<u8>> {
    if data_with_trailer.len() < BLOCK_TRAILER_SIZE {
        return Err(StorKvError::Corruption(
            "Block too short for trailer".into(),
        ));
    }
    let payload_length = data_with_trailer.len() - BLOCK_TRAILER_SIZE;
    let stored_payload = &data_with_trailer[..payload_length];
    let trailer = &data_with_trailer[payload_length..];

    let compression = CompressionType::from_u8(trailer[0])?;
    let expected_checksum = u32::from_le_bytes(
        trailer[1..5]
            .try_into()
            .map_err(|_| StorKvError::Corruption("Invalid block CRC field".into()))?,
    );

    let mut checksum_input = Vec::with_capacity(payload_length + 1);
    checksum_input.extend_from_slice(stored_payload);
    checksum_input.push(trailer[0]);
    let computed_checksum = compute_crc32c(&checksum_input);

    if computed_checksum != expected_checksum {
        return Err(StorKvError::Corruption(format!(
            "Block CRC mismatch: expected 0x{:08x}, got 0x{:08x}",
            expected_checksum, computed_checksum
        )));
    }

    decompress_payload(stored_payload, compression)
}

fn decompress_payload(stored_payload: &[u8], compression: CompressionType) -> Result<Vec<u8>> {
    match compression {
        CompressionType::None => Ok(stored_payload.to_vec()),
        CompressionType::Lz4 => lz4_flex::decompress_size_prepended(stored_payload)
            .map_err(|error| StorKvError::Corruption(error.to_string())),
        CompressionType::Zstd => {
            if stored_payload.len() < 4 {
                return Err(StorKvError::Corruption("Truncated Zstd header".into()));
            }
            zstd::stream::decode_all(&stored_payload[4..])
                .map_err(|error| StorKvError::Corruption(error.to_string()))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Footer {
    pub meta_handle: BlockHandle,
    pub filter_handle: BlockHandle,
    pub index_handle: BlockHandle,
}

impl Footer {
    pub fn encode(&self) -> [u8; FOOTER_SIZE] {
        let mut buffer = [0u8; FOOTER_SIZE];
        let mut cursor = &mut buffer[..];
        self.meta_handle.encode(&mut cursor);
        self.filter_handle.encode(&mut cursor);
        self.index_handle.encode(&mut cursor);
        cursor.put_slice(SST_MAGIC);
        buffer
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < FOOTER_SIZE {
            return Err(StorKvError::Corruption("Footer size too small".into()));
        }
        let footer_bytes = &bytes[bytes.len() - FOOTER_SIZE..];
        if &footer_bytes[48..56] != SST_MAGIC {
            return Err(StorKvError::Corruption(
                "Invalid SSTable magic in footer".into(),
            ));
        }
        let mut cursor = &footer_bytes[0..48];
        Ok(Self {
            meta_handle: BlockHandle::decode(&mut cursor)?,
            filter_handle: BlockHandle::decode(&mut cursor)?,
            index_handle: BlockHandle::decode(&mut cursor)?,
        })
    }
}
