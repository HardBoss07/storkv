use super::block::{DataBlockBuilder, TARGET_BLOCK_SIZE};
use super::bloom::BloomFilterBuilder;
use super::footer::{
    BLOCK_TRAILER_SIZE, BlockHandle, CompressionType, Footer, compress_and_frame_block,
};
use super::meta::{IndexBlockBuilder, MetaBlock};
use crate::error::{Result, StorkvError};
use crate::vfs::File;
use bytes::Bytes;

/// Streaming writer for one SSTable file.
pub struct SstWriter<'a> {
    file: &'a mut Box<dyn File>,
    offset: u64,
    prefix_bloom_len: Option<usize>,
    current_block: Option<DataBlockBuilder>,
    index_builder: IndexBlockBuilder,
    filter_builder: BloomFilterBuilder,
    meta: MetaBlock,
    min_key: Option<Bytes>,
    max_key: Option<Bytes>,
    compression: CompressionType,
}

impl<'a> SstWriter<'a> {
    /// Create a writer that appends to the given file.
    pub fn new(
        file: &'a mut Box<dyn File>,
        scope_id: u32,
        level: u8,
        compression: CompressionType,
        prefix_bloom_len: Option<usize>,
    ) -> Self {
        let meta = MetaBlock {
            scope_id,
            level,
            min_seq: u64::MAX,
            ..Default::default()
        };

        Self {
            file,
            offset: 0,
            current_block: Some(DataBlockBuilder::new()),
            index_builder: IndexBlockBuilder::new(),
            filter_builder: BloomFilterBuilder::new(),
            meta,
            min_key: None,
            max_key: None,
            compression,
            prefix_bloom_len,
        }
    }

    /// Append one entry. Keys must arrive in internal key order.
    ///
    /// # Errors
    ///
    /// Returns an error when a block cannot be compressed or written.
    pub fn add(
        &mut self,
        internal_key: &[u8],
        user_key: &[u8],
        seq: u64,
        expires_at_ms: u64,
        is_tombstone: bool,
        value: &[u8],
    ) -> Result<()> {
        self.update_key_bounds(internal_key);
        self.update_metadata(internal_key, value, seq, expires_at_ms, is_tombstone);
        self.add_to_filter(user_key);

        let builder = self.current_block.as_mut().ok_or_else(|| {
            StorkvError::InvalidOperation("SSTable writer has no active data block".into())
        })?;
        builder.add(internal_key, value);

        if builder.estimated_size() >= TARGET_BLOCK_SIZE {
            self.flush_data_block()?;
        }
        Ok(())
    }

    fn update_key_bounds(&mut self, internal_key: &[u8]) {
        if self.min_key.is_none() {
            self.min_key = Some(Bytes::copy_from_slice(internal_key));
        }
        self.max_key = Some(Bytes::copy_from_slice(internal_key));
    }

    fn update_metadata(
        &mut self,
        internal_key: &[u8],
        value: &[u8],
        seq: u64,
        expires_at_ms: u64,
        is_tombstone: bool,
    ) {
        self.meta.entry_count += 1;
        if is_tombstone {
            self.meta.tombstone_count += 1;
        }
        self.meta.min_seq = self.meta.min_seq.min(seq);
        self.meta.max_seq = self.meta.max_seq.max(seq);
        if expires_at_ms > 0 {
            if self.meta.min_expires_at_ms == 0 {
                self.meta.min_expires_at_ms = expires_at_ms;
            } else {
                self.meta.min_expires_at_ms = self.meta.min_expires_at_ms.min(expires_at_ms);
            }
            self.meta.max_expires_at_ms = self.meta.max_expires_at_ms.max(expires_at_ms);
        }
        self.meta.raw_key_bytes += internal_key.len() as u64;
        self.meta.raw_val_bytes += value.len() as u64;
    }

    fn add_to_filter(&mut self, user_key: &[u8]) {
        self.filter_builder.add_key(user_key);
        if let Some(prefix_length) = self.prefix_bloom_len {
            if user_key.len() >= prefix_length {
                self.filter_builder.add_key(&user_key[..prefix_length]);
            }
        }
    }

    fn flush_data_block(&mut self) -> Result<()> {
        if let Some(builder) = self.current_block.take() {
            self.append_block(builder)?;
            self.current_block = Some(DataBlockBuilder::new());
        }
        Ok(())
    }

    fn append_block(&mut self, builder: DataBlockBuilder) -> Result<()> {
        let last_key = Bytes::copy_from_slice(&builder.last_key);
        let uncompressed = builder.finish();
        let framed = compress_and_frame_block(&uncompressed, self.compression)?;
        let handle = BlockHandle {
            offset: self.offset,
            size: (framed.len() - BLOCK_TRAILER_SIZE) as u64,
        };

        self.write_framed_block(&framed)?;
        self.index_builder.add(last_key, handle);
        Ok(())
    }

    fn write_framed_block(&mut self, framed: &[u8]) -> Result<()> {
        self.file.write_all(framed)?;
        self.offset += framed.len() as u64;
        Ok(())
    }

    fn write_metadata_block(
        file: &mut Box<dyn File>,
        offset: &mut u64,
        bytes: &[u8],
    ) -> Result<BlockHandle> {
        let framed = compress_and_frame_block(bytes, CompressionType::None)?;
        let handle = BlockHandle {
            offset: *offset,
            size: (framed.len() - BLOCK_TRAILER_SIZE) as u64,
        };
        file.write_all(&framed)?;
        *offset += framed.len() as u64;
        Ok(handle)
    }

    /// Flush the last block, write the filter, meta, and index blocks, then
    /// write and sync the footer.
    ///
    /// # Errors
    ///
    /// Returns an error when a block cannot be written or synced.
    pub fn finish(mut self) -> Result<(Footer, MetaBlock, Bytes, Bytes)> {
        self.flush_final_block()?;
        if self.meta.min_seq == u64::MAX {
            self.meta.min_seq = 0;
        }

        let filter_bytes = self.filter_builder.build();
        let filter_handle = Self::write_metadata_block(self.file, &mut self.offset, &filter_bytes)?;

        let meta_bytes = self.meta.encode();
        let meta_handle = Self::write_metadata_block(self.file, &mut self.offset, &meta_bytes)?;

        let index_bytes = self.index_builder.finish();
        let index_handle = Self::write_metadata_block(self.file, &mut self.offset, &index_bytes)?;

        let footer = Footer {
            meta_handle,
            filter_handle,
            index_handle,
        };
        self.file.write_all(&footer.encode())?;
        self.file.sync_all()?;

        Ok((
            footer,
            self.meta,
            self.min_key.unwrap_or_default(),
            self.max_key.unwrap_or_default(),
        ))
    }

    fn flush_final_block(&mut self) -> Result<()> {
        if let Some(builder) = self.current_block.take() {
            if !builder.last_key.is_empty() {
                self.append_block(builder)?;
            }
        }
        Ok(())
    }
}
