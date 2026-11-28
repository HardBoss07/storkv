//! SSTable footer, block handles, and compression re-exports.
//!
//! The footer is a fixed 56-byte record at the end of every SSTable file. It
//! holds the meta, filter, and index block handles, then the format magic.

use crate::error::{Result, StorkvError};
use bytes::{Buf, BufMut};

mod compression;

pub use compression::{CompressionType, compress_and_frame_block, decompress_and_verify_block};

/// Magic bytes at the end of every SSTable file.
pub const SST_MAGIC: &[u8; 8] = b"STORKSST";
/// Size of the fixed SSTable footer in bytes.
pub const FOOTER_SIZE: usize = 56;
/// Size of the compression and checksum trailer on each block.
pub const BLOCK_TRAILER_SIZE: usize = 5;
/// Encoded size of a [`BlockHandle`] in bytes.
pub const BLOCK_HANDLE_SIZE: usize = 16;

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

    /// Decode a handle. The encoded handle is `BLOCK_HANDLE_SIZE` bytes.
    ///
    /// # Errors
    ///
    /// Returns `StorkvError::Corruption` when fewer than `BLOCK_HANDLE_SIZE`
    /// bytes are present.
    pub fn decode(buffer: &mut impl Buf) -> Result<Self> {
        if buffer.remaining() < BLOCK_HANDLE_SIZE {
            return Err(StorkvError::Corruption("Incomplete BlockHandle".into()));
        }
        Ok(Self {
            offset: buffer.get_u64_le(),
            size: buffer.get_u64_le(),
        })
    }
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
    /// Returns `StorkvError::Corruption` on a short file or bad magic.
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
