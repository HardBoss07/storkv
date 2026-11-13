use crate::error::{Result, StorKvError};
use crate::format::compute_crc32c;
use crate::vfs::File;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::io::SeekFrom;

pub const SST_MAGIC: &[u8; 8] = b"STORKSST";
pub const FOOTER_SIZE: usize = 56;
pub const BLOCK_TRAILER_SIZE: usize = 5;
pub const RESTART_INTERVAL: usize = 16;
pub const TARGET_BLOCK_SIZE: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockHandle {
    pub offset: u64,
    pub size: u64, // on-disk stored byte length, excluding 5-byte trailer
}

impl BlockHandle {
    pub fn encode(&self, buf: &mut impl BufMut) {
        buf.put_u64_le(self.offset);
        buf.put_u64_le(self.size);
    }

    pub fn decode(buf: &mut impl Buf) -> Result<Self> {
        if buf.remaining() < 16 {
            return Err(StorKvError::Corruption("Incomplete BlockHandle".into()));
        }
        Ok(Self {
            offset: buf.get_u64_le(),
            size: buf.get_u64_le(),
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
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0x00 => Ok(CompressionType::None),
            0x01 => Ok(CompressionType::Lz4),
            0x02 => Ok(CompressionType::Zstd),
            _ => Err(StorKvError::Corruption(format!(
                "Unknown compression type: {}",
                v
            ))),
        }
    }
}

/// Compress payload and format with 5-byte trailer
pub fn compress_and_frame_block(payload: &[u8], compression: CompressionType) -> Result<Vec<u8>> {
    let mut stored_payload = match compression {
        CompressionType::None => payload.to_vec(),
        CompressionType::Lz4 => {
            let compressed = lz4_flex::compress_prepend_size(payload);
            let mut buf = Vec::with_capacity(4 + compressed.len());
            buf.extend_from_slice(&compressed);
            buf
        }
        CompressionType::Zstd => {
            let mut out = Vec::new();
            out.put_u32_le(payload.len() as u32);
            let compressed = zstd::stream::encode_all(payload, 0)
                .map_err(|e| StorKvError::Corruption(e.to_string()))?;
            out.extend_from_slice(&compressed);
            out
        }
    };

    // Calculate CRC32C over stored block payload + compression_type byte
    let mut crc_data = Vec::with_capacity(stored_payload.len() + 1);
    crc_data.extend_from_slice(&stored_payload);
    crc_data.push(compression as u8);
    let crc = compute_crc32c(&crc_data);

    // Append 5-byte trailer: compression_type (1) + crc (4 LE)
    stored_payload.push(compression as u8);
    stored_payload.extend_from_slice(&crc.to_le_bytes());

    Ok(stored_payload)
}

/// Read, verify checksum, and decompress block payload
pub fn decompress_and_verify_block(data_with_trailer: &[u8]) -> Result<Vec<u8>> {
    if data_with_trailer.len() < BLOCK_TRAILER_SIZE {
        return Err(StorKvError::Corruption(
            "Block too short for trailer".into(),
        ));
    }
    let stored_payload_len = data_with_trailer.len() - BLOCK_TRAILER_SIZE;
    let stored_payload = &data_with_trailer[..stored_payload_len];
    let trailer = &data_with_trailer[stored_payload_len..];

    let comp_type = CompressionType::from_u8(trailer[0])?;
    let expected_crc = u32::from_le_bytes(trailer[1..5].try_into().unwrap());

    let mut crc_data = Vec::with_capacity(stored_payload_len + 1);
    crc_data.extend_from_slice(stored_payload);
    crc_data.push(trailer[0]);
    let computed_crc = compute_crc32c(&crc_data);

    if computed_crc != expected_crc {
        return Err(StorKvError::Corruption(format!(
            "Block CRC mismatch: expected 0x{:08x}, got 0x{:08x}",
            expected_crc, computed_crc
        )));
    }

    match comp_type {
        CompressionType::None => Ok(stored_payload.to_vec()),
        CompressionType::Lz4 => {
            let decompressed = lz4_flex::decompress_size_prepended(stored_payload)
                .map_err(|e| StorKvError::Corruption(e.to_string()))?;
            Ok(decompressed)
        }
        CompressionType::Zstd => {
            if stored_payload.len() < 4 {
                return Err(StorKvError::Corruption("Truncated Zstd header".into()));
            }
            let decompressed = zstd::stream::decode_all(&stored_payload[4..])
                .map_err(|e| StorKvError::Corruption(e.to_string()))?;
            Ok(decompressed)
        }
    }
}

/// Blocked Prefix Bloom Filter (num_probes = 7, bits_per_key = 10)
pub struct BloomFilterBuilder {
    bits_per_key: usize,
    num_probes: u8,
    hashes: Vec<u32>,
}

impl BloomFilterBuilder {
    pub fn new() -> Self {
        Self {
            bits_per_key: 10,
            num_probes: 7,
            hashes: Vec::new(),
        }
    }

    pub fn add_key(&mut self, key: &[u8]) {
        self.hashes.push(compute_crc32c(key));
    }

    pub fn build(self) -> Vec<u8> {
        let bitset_bits = if self.hashes.is_empty() {
            64
        } else {
            (self.hashes.len() * self.bits_per_key).max(64)
        };
        let bitset_bytes = bitset_bits.div_ceil(8);
        let mut bitset = vec![0u8; bitset_bytes];

        for h in self.hashes {
            let mut hash = h;
            let delta = hash.rotate_left(15);
            for _ in 0..self.num_probes {
                let bit_pos = (hash as usize) % (bitset_bytes * 8);
                bitset[bit_pos / 8] |= 1 << (bit_pos % 8);
                hash = hash.wrapping_add(delta);
            }
        }

        // Header: num_probes (1), bits_per_key (1), reserved (2), bitset_len (4 LE)
        let mut out = Vec::with_capacity(8 + bitset_bytes);
        out.push(self.num_probes);
        out.push(self.bits_per_key as u8);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(bitset_bytes as u32).to_le_bytes());
        out.extend_from_slice(&bitset);
        out
    }
}

pub struct BloomFilterReader {
    num_probes: u8,
    bitset: Bytes,
}

impl BloomFilterReader {
    pub fn new(data: Bytes) -> Result<Self> {
        if data.len() < 8 {
            return Err(StorKvError::Corruption("Filter block too short".into()));
        }
        let num_probes = data[0];
        let bitset_len = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        if data.len() < 8 + bitset_len {
            return Err(StorKvError::Corruption("Truncated filter bitset".into()));
        }
        let bitset = data.slice(8..8 + bitset_len);
        Ok(Self { num_probes, bitset })
    }

    pub fn may_contain(&self, key: &[u8]) -> bool {
        if self.bitset.is_empty() {
            return true;
        }
        let total_bits = self.bitset.len() * 8;
        let mut hash = compute_crc32c(key);
        let delta = hash.rotate_left(15);
        for _ in 0..self.num_probes {
            let bit_pos = (hash as usize) % total_bits;
            if (self.bitset[bit_pos / 8] & (1 << (bit_pos % 8))) == 0 {
                return false;
            }
            hash = hash.wrapping_add(delta);
        }
        true
    }
}

/// Meta Block Layout (69 Bytes + 5-Byte Trailer)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetaBlock {
    pub scope_id: u32,
    pub level: u8,
    pub entry_count: u64,
    pub tombstone_count: u64,
    pub min_seq: u64,
    pub max_seq: u64,
    pub min_expires_at_ms: u64,
    pub max_expires_at_ms: u64,
    pub raw_key_bytes: u64,
    pub raw_val_bytes: u64,
}

impl MetaBlock {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = BytesMut::with_capacity(69);
        buf.put_u32_le(self.scope_id);
        buf.put_u8(self.level);
        buf.put_u64_le(self.entry_count);
        buf.put_u64_le(self.tombstone_count);
        buf.put_u64_le(self.min_seq);
        buf.put_u64_le(self.max_seq);
        buf.put_u64_le(self.min_expires_at_ms);
        buf.put_u64_le(self.max_expires_at_ms);
        buf.put_u64_le(self.raw_key_bytes);
        buf.put_u64_le(self.raw_val_bytes);
        buf.to_vec()
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

/// Data Block Builder with Prefix Compression & Restart Points every 16 entries
pub struct DataBlockBuilder {
    buf: BytesMut,
    restarts: Vec<u32>,
    last_key: Vec<u8>,
    counter: usize,
}

impl DataBlockBuilder {
    pub fn new() -> Self {
        Self {
            buf: BytesMut::new(),
            restarts: vec![0],
            last_key: Vec::new(),
            counter: 0,
        }
    }

    pub fn add(&mut self, key: &[u8], value: &[u8]) {
        let mut shared = 0;
        if self.counter.is_multiple_of(RESTART_INTERVAL) {
            if self.counter > 0 {
                self.restarts.push(self.buf.len() as u32);
            }
        } else {
            while shared < key.len()
                && shared < self.last_key.len()
                && key[shared] == self.last_key[shared]
            {
                shared += 1;
            }
        }

        let unshared = key.len() - shared;
        self.buf.put_u16_le(shared as u16);
        self.buf.put_u16_le(unshared as u16);
        self.buf.put_u32_le(value.len() as u32);
        self.buf.extend_from_slice(&key[shared..]);
        self.buf.extend_from_slice(value);

        self.last_key.clear();
        self.last_key.extend_from_slice(key);
        self.counter += 1;
    }

    pub fn estimated_size(&self) -> usize {
        self.buf.len() + (self.restarts.len() * 4) + 4
    }

    pub fn finish(mut self) -> Vec<u8> {
        for &offset in &self.restarts {
            self.buf.put_u32_le(offset);
        }
        self.buf.put_u32_le(self.restarts.len() as u32);
        self.buf.to_vec()
    }
}

/// Data Block Reader / Iterator
pub struct DataBlockReader {
    data: Bytes,
    restarts: Vec<u32>,
}

impl DataBlockReader {
    pub fn new(data: Bytes) -> Result<Self> {
        if data.len() < 4 {
            return Err(StorKvError::Corruption("Data block too short".into()));
        }
        let num_restarts = u32::from_le_bytes(data[data.len() - 4..].try_into().unwrap()) as usize;
        let restart_array_len = num_restarts * 4;
        if data.len() < 4 + restart_array_len {
            return Err(StorKvError::Corruption(
                "Corrupted restart array in data block".into(),
            ));
        }
        let restart_offset_start = data.len() - 4 - restart_array_len;
        let mut restarts = Vec::with_capacity(num_restarts);
        for i in 0..num_restarts {
            let pos = restart_offset_start + i * 4;
            let offset = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
            restarts.push(offset);
        }

        Ok(Self { data, restarts })
    }

    pub fn entries(&self) -> Result<Vec<(Bytes, Bytes)>> {
        let restart_section_start = self.restarts[0] as usize;
        let mut offset = restart_section_start;
        let end = self.data.len() - 4 - (self.restarts.len() * 4);

        let mut entries = Vec::new();
        let mut current_key = Vec::new();

        while offset < end {
            if offset + 8 > end {
                return Err(StorKvError::Corruption("Truncated entry header".into()));
            }
            let shared =
                u16::from_le_bytes(self.data[offset..offset + 2].try_into().unwrap()) as usize;
            let unshared =
                u16::from_le_bytes(self.data[offset + 2..offset + 4].try_into().unwrap()) as usize;
            let val_len =
                u32::from_le_bytes(self.data[offset + 4..offset + 8].try_into().unwrap()) as usize;
            offset += 8;

            if offset + unshared + val_len > end {
                return Err(StorKvError::Corruption("Truncated entry key/val".into()));
            }

            current_key.truncate(shared);
            current_key.extend_from_slice(&self.data[offset..offset + unshared]);
            offset += unshared;

            let val = self.data.slice(offset..offset + val_len);
            offset += val_len;

            entries.push((Bytes::copy_from_slice(&current_key), val));
        }

        Ok(entries)
    }

    /// Binary search within the block using restart array, then linear scan
    pub fn seek_ge(&self, target_internal_key: &[u8]) -> Result<Option<(Bytes, Bytes)>> {
        // Linear scan over full entries for simplicity and complete correctness
        let entries = self.entries()?;
        for (k, v) in entries {
            if k.as_ref() >= target_internal_key {
                return Ok(Some((k, v)));
            }
        }
        Ok(None)
    }
}

/// Index Block Entry: (last_internal_key, BlockHandle)
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
        let mut buf = BytesMut::new();
        buf.put_u32_le(self.entries.len() as u32);
        for entry in self.entries {
            buf.put_u16_le(entry.last_internal_key.len() as u16);
            buf.extend_from_slice(&entry.last_internal_key);
            entry.handle.encode(&mut buf);
        }
        buf.to_vec()
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
        let count = data.get_u32_le() as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if data.remaining() < 2 {
                return Err(StorKvError::Corruption("Truncated index key length".into()));
            }
            let key_len = data.get_u16_le() as usize;
            if data.remaining() < key_len + 16 {
                return Err(StorKvError::Corruption("Truncated index entry".into()));
            }
            let mut key_bytes = vec![0u8; key_len];
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
        self.entries.last().map(|e| e.handle)
    }
}

/// SSTable Footer (56 Bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Footer {
    pub meta_handle: BlockHandle,
    pub filter_handle: BlockHandle,
    pub index_handle: BlockHandle,
}

impl Footer {
    pub fn encode(&self) -> [u8; FOOTER_SIZE] {
        let mut buf = [0u8; FOOTER_SIZE];
        let mut cur = &mut buf[..];
        self.meta_handle.encode(&mut cur);
        self.filter_handle.encode(&mut cur);
        self.index_handle.encode(&mut cur);
        cur.put_slice(SST_MAGIC);
        buf
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < FOOTER_SIZE {
            return Err(StorKvError::Corruption("Footer size too small".into()));
        }
        let slice = &bytes[bytes.len() - FOOTER_SIZE..];
        if &slice[48..56] != SST_MAGIC {
            return Err(StorKvError::Corruption(
                "Invalid SSTable magic in footer".into(),
            ));
        }
        let mut cur = &slice[0..48];
        let meta_handle = BlockHandle::decode(&mut cur)?;
        let filter_handle = BlockHandle::decode(&mut cur)?;
        let index_handle = BlockHandle::decode(&mut cur)?;
        Ok(Self {
            meta_handle,
            filter_handle,
            index_handle,
        })
    }
}

/// SSTable Builder for creating immutable `.sst` files
pub struct SstWriter<'a> {
    file: &'a mut Box<dyn File>,
    offset: u64,
    current_block: Option<DataBlockBuilder>,
    index_builder: IndexBlockBuilder,
    filter_builder: BloomFilterBuilder,
    meta: MetaBlock,
    min_key: Option<Bytes>,
    max_key: Option<Bytes>,
    compression: CompressionType,
    prefix_bloom_len: Option<usize>,
}

impl<'a> SstWriter<'a> {
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

    pub fn add(
        &mut self,
        internal_key: &[u8],
        user_key: &[u8],
        seq: u64,
        expires_at_ms: u64,
        is_tombstone: bool,
        value: &[u8],
    ) -> Result<()> {
        if self.min_key.is_none() {
            self.min_key = Some(Bytes::copy_from_slice(internal_key));
        }
        self.max_key = Some(Bytes::copy_from_slice(internal_key));

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

        // Add to bloom filter: full user key and optional prefix
        self.filter_builder.add_key(user_key);
        if let Some(prefix_len) = self.prefix_bloom_len {
            if user_key.len() >= prefix_len {
                self.filter_builder.add_key(&user_key[..prefix_len]);
            }
        }

        let builder = self.current_block.as_mut().unwrap();
        builder.add(internal_key, value);

        if builder.estimated_size() >= TARGET_BLOCK_SIZE {
            self.flush_data_block()?;
        }

        Ok(())
    }

    fn flush_data_block(&mut self) -> Result<()> {
        if let Some(builder) = self.current_block.take() {
            let last_key = Bytes::copy_from_slice(&builder.last_key);
            let uncompressed = builder.finish();
            let framed = compress_and_frame_block(&uncompressed, self.compression)?;
            let size = (framed.len() - BLOCK_TRAILER_SIZE) as u64;
            let handle = BlockHandle {
                offset: self.offset,
                size,
            };

            self.file.write_all(&framed)?;
            self.offset += framed.len() as u64;
            self.index_builder.add(last_key, handle);
            self.current_block = Some(DataBlockBuilder::new());
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<(Footer, MetaBlock, Bytes, Bytes)> {
        if let Some(builder) = self.current_block.take() {
            if !builder.last_key.is_empty() {
                let last_key = Bytes::copy_from_slice(&builder.last_key);
                let uncompressed = builder.finish();
                let framed = compress_and_frame_block(&uncompressed, self.compression)?;
                let size = (framed.len() - BLOCK_TRAILER_SIZE) as u64;
                let handle = BlockHandle {
                    offset: self.offset,
                    size,
                };
                self.file.write_all(&framed)?;
                self.offset += framed.len() as u64;
                self.index_builder.add(last_key, handle);
            }
        }

        if self.meta.min_seq == u64::MAX {
            self.meta.min_seq = 0;
        }

        // Write Filter Block
        let filter_bytes = self.filter_builder.build();
        let framed_filter = compress_and_frame_block(&filter_bytes, CompressionType::None)?;
        let filter_handle = BlockHandle {
            offset: self.offset,
            size: (framed_filter.len() - BLOCK_TRAILER_SIZE) as u64,
        };
        self.file.write_all(&framed_filter)?;
        self.offset += framed_filter.len() as u64;

        // Write Meta Block
        let meta_bytes = self.meta.encode();
        let framed_meta = compress_and_frame_block(&meta_bytes, CompressionType::None)?;
        let meta_handle = BlockHandle {
            offset: self.offset,
            size: (framed_meta.len() - BLOCK_TRAILER_SIZE) as u64,
        };
        self.file.write_all(&framed_meta)?;
        self.offset += framed_meta.len() as u64;

        // Write Index Block
        let index_bytes = self.index_builder.finish();
        let framed_index = compress_and_frame_block(&index_bytes, CompressionType::None)?;
        let index_handle = BlockHandle {
            offset: self.offset,
            size: (framed_index.len() - BLOCK_TRAILER_SIZE) as u64,
        };
        self.file.write_all(&framed_index)?;
        self.offset += framed_index.len() as u64;

        // Write 56-byte Footer
        let footer = Footer {
            meta_handle,
            filter_handle,
            index_handle,
        };
        let footer_bytes = footer.encode();
        self.file.write_all(&footer_bytes)?;
        self.file.sync_all()?;

        let min_key = self.min_key.unwrap_or_default();
        let max_key = self.max_key.unwrap_or_default();
        Ok((footer, self.meta, min_key, max_key))
    }
}

/// SSTable Reader for point queries and range scans
pub struct SstReader {
    pub footer: Footer,
    pub meta: MetaBlock,
    pub index: IndexBlockReader,
    pub filter: BloomFilterReader,
}

impl SstReader {
    pub fn open(file: &mut Box<dyn File>) -> Result<Self> {
        let file_len = file.file_size()?;
        if file_len < FOOTER_SIZE as u64 {
            return Err(StorKvError::Corruption(
                "SSTable file smaller than footer".into(),
            ));
        }

        // Read Footer
        file.seek(SeekFrom::Start(file_len - FOOTER_SIZE as u64))?;
        let mut footer_buf = [0u8; FOOTER_SIZE];
        file.read_exact(&mut footer_buf)?;
        let footer = Footer::decode(&footer_buf)?;

        // Read Meta Block
        let meta_buf = Self::read_raw_block(file, footer.meta_handle)?;
        let meta_bytes = decompress_and_verify_block(&meta_buf)?;
        let meta = MetaBlock::decode(&meta_bytes)?;

        // Read Filter Block
        let filter_buf = Self::read_raw_block(file, footer.filter_handle)?;
        let filter_bytes = decompress_and_verify_block(&filter_buf)?;
        let filter = BloomFilterReader::new(Bytes::from(filter_bytes))?;

        // Read Index Block
        let index_buf = Self::read_raw_block(file, footer.index_handle)?;
        let index_bytes = decompress_and_verify_block(&index_buf)?;
        let index = IndexBlockReader::decode(&index_bytes)?;

        Ok(Self {
            footer,
            meta,
            index,
            filter,
        })
    }

    pub fn read_raw_block(file: &mut Box<dyn File>, handle: BlockHandle) -> Result<Vec<u8>> {
        let total_size = handle.size as usize + BLOCK_TRAILER_SIZE;
        file.seek(SeekFrom::Start(handle.offset))?;
        let mut buf = vec![0u8; total_size];
        file.read_exact(&mut buf)?;
        Ok(buf)
    }

    pub fn read_data_block(
        &self,
        file: &mut Box<dyn File>,
        handle: BlockHandle,
    ) -> Result<DataBlockReader> {
        let raw = Self::read_raw_block(file, handle)?;
        let decompressed = decompress_and_verify_block(&raw)?;
        DataBlockReader::new(Bytes::from(decompressed))
    }

    pub fn read_all_entries(&self, file: &mut Box<dyn File>) -> Result<Vec<(Bytes, Bytes)>> {
        let mut results = Vec::new();
        for entry in &self.index.entries {
            let reader = self.read_data_block(file, entry.handle)?;
            results.extend(reader.entries()?);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{InternalKey, ValueType};
    use crate::vfs::{FileSystem, PosixFs};
    use std::sync::Arc;

    #[test]
    fn test_sstable_build_and_read() {
        let temp = std::env::temp_dir().join(format!("storkv_test_sst_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let fs = Arc::new(PosixFs);
        fs.create_dir_all(&temp).unwrap();
        let sst_path = temp.join("0000000000000001.sst");

        {
            let mut file = fs.create_file(&sst_path).unwrap();
            let mut writer = SstWriter::new(&mut file, 0, 0, CompressionType::Lz4, Some(4));

            for i in 0..100 {
                let user_key = format!("user:{:05}", i).into_bytes();
                let ikey = InternalKey::new(
                    Bytes::copy_from_slice(&user_key),
                    100 + i,
                    ValueType::Value,
                    0,
                );
                let enc_ikey = ikey.encode();
                let val = format!("val:{:05}", i).into_bytes();
                writer
                    .add(&enc_ikey, &user_key, 100 + i, 0, false, &val)
                    .unwrap();
            }

            let (_footer, meta, _, _) = writer.finish().unwrap();
            assert_eq!(meta.entry_count, 100);
            assert_eq!(meta.min_seq, 100);
            assert_eq!(meta.max_seq, 199);
        }

        {
            let mut file = fs.open_file(&sst_path).unwrap();
            let reader = SstReader::open(&mut file).unwrap();
            assert_eq!(reader.meta.entry_count, 100);

            // Test Bloom filter
            assert!(reader.filter.may_contain(b"user:00050"));
            assert!(!reader.filter.may_contain(b"non_existent_key_12345"));

            // Read all entries
            let entries = reader.read_all_entries(&mut file).unwrap();
            assert_eq!(entries.len(), 100);

            let (first_k, first_v) = &entries[0];
            let dec = InternalKey::decode(first_k.clone()).unwrap();
            assert_eq!(dec.user_key.as_ref(), b"user:00000");
            assert_eq!(first_v.as_ref(), b"val:00000");
        }

        let _ = std::fs::remove_dir_all(&temp);
    }
}
