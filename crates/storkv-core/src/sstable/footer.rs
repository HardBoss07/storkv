use std::io::Read;

use crate::error::{Result, StorkvError};
use crate::format::{MAX_VAL_LEN, compute_crc32c};
use bytes::{Buf, BufMut};

/// Magic bytes at the end of every SSTable file.
pub const SST_MAGIC: &[u8; 8] = b"STORKSST";
/// Size of the fixed SSTable footer in bytes.
pub const FOOTER_SIZE: usize = 56;
/// Size of the compression and checksum trailer on each block.
pub const BLOCK_TRAILER_SIZE: usize = 5;

/// Offset and size of one SSTable block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockHandle {
    /// Byte offset from the start of the file.
    pub offset: u64,
    /// Block size in bytes, including the trailer.
    pub size: u64,
}

impl BlockHandle {
    /// Encode the handle in little-endian order.
    pub fn encode(&self, buffer: &mut impl BufMut) {
        buffer.put_u64_le(self.offset);
        buffer.put_u64_le(self.size);
    }

    /// Decode a handle from 16 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::Corruption`] when fewer than 16 bytes are present.
    pub fn decode(buffer: &mut impl Buf) -> Result<Self> {
        if buffer.remaining() < 16 {
            return Err(StorkvError::Corruption("Incomplete BlockHandle".into()));
        }
        Ok(Self {
            offset: buffer.get_u64_le(),
            size: buffer.get_u64_le(),
        })
    }
}

/// Compression codec of one SSTable block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompressionType {
    /// No compression.
    None = 0x00,
    /// LZ4 block compression.
    Lz4 = 0x01,
    /// Zstandard compression.
    Zstd = 0x02,
}

impl CompressionType {
    /// Decode a compression code.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::Corruption`] for an unknown code.
    pub fn from_u8(value: u8) -> Result<Self> {
        match value {
            0x00 => Ok(CompressionType::None),
            0x01 => Ok(CompressionType::Lz4),
            0x02 => Ok(CompressionType::Zstd),
            _ => Err(StorkvError::Corruption(format!(
                "Unknown compression type: {}",
                value
            ))),
        }
    }
}

/// Compress a block and append the compression code and CRC32C trailer.
///
/// # Errors
///
/// Returns [`StorkvError::Corruption`] when the codec fails.
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
                .map_err(|error| StorkvError::Corruption(error.to_string()))?;
            compressed.extend_from_slice(&encoded);
            Ok(compressed)
        }
    }
}

/// Verify the CRC32C trailer and decompress a block.
///
/// # Errors
///
/// Returns [`StorkvError::Corruption`] on a short block, a checksum mismatch,
/// or a decompression failure.
pub fn decompress_and_verify_block(data_with_trailer: &[u8]) -> Result<Vec<u8>> {
    if data_with_trailer.len() < BLOCK_TRAILER_SIZE {
        return Err(StorkvError::Corruption(
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
            .map_err(|_| StorkvError::Corruption("Invalid block CRC field".into()))?,
    );

    let mut checksum_input = Vec::with_capacity(payload_length + 1);
    checksum_input.extend_from_slice(stored_payload);
    checksum_input.push(trailer[0]);
    let computed_checksum = compute_crc32c(&checksum_input);

    if computed_checksum != expected_checksum {
        return Err(StorkvError::Corruption(format!(
            "Block CRC mismatch: expected 0x{:08x}, got 0x{:08x}",
            expected_checksum, computed_checksum
        )));
    }

    decompress_payload(stored_payload, compression)
}

fn decompress_payload(stored_payload: &[u8], compression: CompressionType) -> Result<Vec<u8>> {
    match compression {
        CompressionType::None => Ok(stored_payload.to_vec()),
        CompressionType::Lz4 => decompress_lz4_payload(stored_payload),
        CompressionType::Zstd => decompress_zstd_payload(stored_payload),
    }
}

fn declared_payload_len(stored_payload: &[u8], label: &str) -> Result<usize> {
    if stored_payload.len() < 4 {
        return Err(StorkvError::Corruption(format!("Truncated {label} header")));
    }
    let declared = u32::from_le_bytes(
        stored_payload[..4]
            .try_into()
            .map_err(|_| StorkvError::Corruption(format!("Invalid {label} header")))?,
    ) as usize;
    if declared > MAX_VAL_LEN {
        return Err(StorkvError::Corruption(format!(
            "{label} payload length {declared} exceeds the limit {MAX_VAL_LEN}"
        )));
    }
    Ok(declared)
}

fn decompress_lz4_payload(stored_payload: &[u8]) -> Result<Vec<u8>> {
    let declared = declared_payload_len(stored_payload, "Lz4")?;
    lz4_flex::decompress(&stored_payload[4..], declared)
        .map_err(|error| StorkvError::Corruption(error.to_string()))
}

fn decompress_zstd_payload(stored_payload: &[u8]) -> Result<Vec<u8>> {
    let declared = declared_payload_len(stored_payload, "Zstd")?;
    let decoder = zstd::stream::read::Decoder::new(&stored_payload[4..])
        .map_err(|error| StorkvError::Corruption(error.to_string()))?;
    let mut decoded = Vec::with_capacity(declared);
    decoder
        .take((MAX_VAL_LEN + 1) as u64)
        .read_to_end(&mut decoded)
        .map_err(|error| StorkvError::Corruption(error.to_string()))?;
    if decoded.len() > MAX_VAL_LEN {
        return Err(StorkvError::Corruption(format!(
            "Zstd payload exceeds the limit {MAX_VAL_LEN}"
        )));
    }
    if decoded.len() != declared {
        return Err(StorkvError::Corruption(format!(
            "Zstd payload length mismatch: declared {declared}, decoded {}",
            decoded.len()
        )));
    }
    Ok(decoded)
}

/// Fixed SSTable footer: handles of the meta, filter, and index blocks plus
/// the format magic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Footer {
    /// Handle of the metadata block.
    pub meta_handle: BlockHandle,
    /// Handle of the prefix Bloom filter block.
    pub filter_handle: BlockHandle,
    /// Handle of the index block.
    pub index_handle: BlockHandle,
}

impl Footer {
    /// Encode the footer to exactly [`FOOTER_SIZE`] bytes.
    pub fn encode(&self) -> [u8; FOOTER_SIZE] {
        let mut buffer = [0u8; FOOTER_SIZE];
        let mut cursor = &mut buffer[..];
        self.meta_handle.encode(&mut cursor);
        self.filter_handle.encode(&mut cursor);
        self.index_handle.encode(&mut cursor);
        cursor.put_slice(SST_MAGIC);
        buffer
    }

    /// Decode the last [`FOOTER_SIZE`] bytes of an SSTable file.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::Corruption`] on a short file or bad magic.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < FOOTER_SIZE {
            return Err(StorkvError::Corruption("Footer size too small".into()));
        }
        let footer_bytes = &bytes[bytes.len() - FOOTER_SIZE..];
        if &footer_bytes[48..56] != SST_MAGIC {
            return Err(StorkvError::Corruption(
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
