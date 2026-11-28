//! SSTable reader and lookup paths.
use super::block::DataBlockReader;
use super::block_cache::{BlockCache, BlockKey};
use super::bloom::BloomFilterReader;
use super::footer::{
    BLOCK_TRAILER_SIZE, BlockHandle, FOOTER_SIZE, Footer, decompress_and_verify_block,
};
use super::meta::{IndexBlockReader, MetaBlock};
use crate::error::{Result, StorkvError};
use crate::vfs::File;
use bytes::Bytes;
use std::io::SeekFrom;

/// Reader for one SSTable file.
pub struct SstReader {
    /// Parsed footer.
    pub footer: Footer,
    /// Parsed metadata block.
    pub meta: MetaBlock,
    /// Parsed index block.
    pub index: IndexBlockReader,
    /// Parsed prefix Bloom filter.
    pub filter: BloomFilterReader,
}

impl SstReader {
    /// Open an SSTable and parse its footer, meta, filter, and index blocks.
    ///
    /// # Errors
    ///
    /// Returns `StorkvError::Corruption` on a short file or invalid block.
    pub fn open(file: &mut Box<dyn File>) -> Result<Self> {
        let file_length = file.file_size()?;
        if file_length < FOOTER_SIZE as u64 {
            return Err(StorkvError::Corruption(
                "SSTable file smaller than footer".into(),
            ));
        }

        let footer = Self::read_footer(file, file_length)?;
        let meta = MetaBlock::decode(&Self::read_decoded_block(file, footer.meta_handle)?)?;

        let filter_bytes = Self::read_decoded_block(file, footer.filter_handle)?;
        let filter = BloomFilterReader::new(Bytes::from(filter_bytes))?;

        let index_bytes = Self::read_decoded_block(file, footer.index_handle)?;
        let index = IndexBlockReader::decode(&index_bytes)?;

        Ok(Self {
            footer,
            meta,
            index,
            filter,
        })
    }

    /// Open an SSTable and read its index block through the shared cache.
    ///
    /// `fill_cache` controls whether a cache miss inserts the index block.
    ///
    /// # Errors
    ///
    /// Returns `StorkvError::Corruption` on a short file or invalid block.
    pub fn open_cached(
        file: &mut Box<dyn File>,
        cache: &BlockCache,
        scope_id: u32,
        sst_id: u64,
        fill_cache: bool,
    ) -> Result<Self> {
        let file_length = file.file_size()?;
        if file_length < FOOTER_SIZE as u64 {
            return Err(StorkvError::Corruption(
                "SSTable file smaller than footer".into(),
            ));
        }

        let footer = Self::read_footer(file, file_length)?;
        let meta = MetaBlock::decode(&Self::read_decoded_block(file, footer.meta_handle)?)?;

        let filter_bytes = Self::read_decoded_block(file, footer.filter_handle)?;
        let filter = BloomFilterReader::new(Bytes::from(filter_bytes))?;

        let index_bytes = read_cached_block(
            file,
            cache,
            BlockKey::new(scope_id, sst_id, footer.index_handle.offset),
            footer.index_handle,
            fill_cache,
        )?;
        let index = IndexBlockReader::decode(&index_bytes)?;

        Ok(Self {
            footer,
            meta,
            index,
            filter,
        })
    }

    fn read_footer(file: &mut Box<dyn File>, file_length: u64) -> Result<Footer> {
        file.seek(SeekFrom::Start(file_length - FOOTER_SIZE as u64))?;
        let mut footer_bytes = [0u8; FOOTER_SIZE];
        file.read_exact(&mut footer_bytes)?;
        Footer::decode(&footer_bytes)
    }

    fn read_decoded_block(file: &mut Box<dyn File>, handle: BlockHandle) -> Result<Vec<u8>> {
        let raw_block = Self::read_raw_block(file, handle)?;
        decompress_and_verify_block(&raw_block)
    }

    /// Read the raw bytes of one block plus its trailer.
    ///
    /// # Errors
    ///
    /// Returns `StorkvError::Corruption` on an overflow or a short read.
    pub fn read_raw_block(file: &mut Box<dyn File>, handle: BlockHandle) -> Result<Vec<u8>> {
        let total_size = usize::try_from(handle.size)
            .ok()
            .and_then(|size| size.checked_add(BLOCK_TRAILER_SIZE))
            .ok_or_else(|| StorkvError::Corruption("Block handle size overflow".into()))?;
        file.seek(SeekFrom::Start(handle.offset))?;
        let mut block_bytes = vec![0u8; total_size];
        file.read_exact(&mut block_bytes)?;
        Ok(block_bytes)
    }

    /// Verify and decompress one data block.
    ///
    /// # Errors
    ///
    /// Returns `StorkvError::Corruption` on a checksum or decode failure.
    pub fn read_data_block(
        &self,
        file: &mut Box<dyn File>,
        handle: BlockHandle,
    ) -> Result<DataBlockReader> {
        let decompressed = Self::read_decoded_block(file, handle)?;
        DataBlockReader::new(Bytes::from(decompressed))
    }

    /// Read one data block through the shared cache.
    ///
    /// `fill_cache` controls whether a cache miss inserts the block.
    ///
    /// # Errors
    ///
    /// Returns `StorkvError::Corruption` on a checksum or decode failure.
    pub fn read_data_block_cached(
        &self,
        file: &mut Box<dyn File>,
        cache: &BlockCache,
        scope_id: u32,
        sst_id: u64,
        handle: BlockHandle,
        fill_cache: bool,
    ) -> Result<DataBlockReader> {
        let decompressed = read_cached_block(
            file,
            cache,
            BlockKey::new(scope_id, sst_id, handle.offset),
            handle,
            fill_cache,
        )?;
        DataBlockReader::new(decompressed)
    }

    /// Read every key/value entry in the file.
    ///
    /// # Errors
    ///
    /// Returns `StorkvError::Corruption` when a block cannot be decoded.
    pub fn read_all_entries(&self, file: &mut Box<dyn File>) -> Result<Vec<(Bytes, Bytes)>> {
        let mut entries = Vec::new();
        for index_entry in &self.index.entries {
            let block_reader = self.read_data_block(file, index_entry.handle)?;
            entries.extend(block_reader.entries()?);
        }
        Ok(entries)
    }

    /// Read every key/value entry through the shared cache.
    ///
    /// # Errors
    ///
    /// Returns `StorkvError::Corruption` when a block cannot be decoded.
    pub fn read_all_entries_cached(
        &self,
        file: &mut Box<dyn File>,
        cache: &BlockCache,
        scope_id: u32,
        sst_id: u64,
        fill_cache: bool,
    ) -> Result<Vec<(Bytes, Bytes)>> {
        let mut entries = Vec::new();
        for index_entry in &self.index.entries {
            let block_reader = self.read_data_block_cached(
                file,
                cache,
                scope_id,
                sst_id,
                index_entry.handle,
                fill_cache,
            )?;
            entries.extend(block_reader.entries()?);
        }
        Ok(entries)
    }
}

fn read_cached_block(
    file: &mut Box<dyn File>,
    cache: &BlockCache,
    key: BlockKey,
    handle: BlockHandle,
    fill_cache: bool,
) -> Result<Bytes> {
    if let Some(cached) = cache.get(&key) {
        return Ok(cached);
    }
    let raw_block = SstReader::read_raw_block(file, handle)?;
    let decompressed = Bytes::from(decompress_and_verify_block(&raw_block)?);
    if fill_cache {
        cache.put(key, decompressed.clone());
    }
    Ok(decompressed)
}
