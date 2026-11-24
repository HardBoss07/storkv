use crate::error::{Result, StorkvError};
use crate::format::compute_crc32c;
use bytes::Bytes;

const DEFAULT_BITS_PER_KEY: usize = 10;
const DEFAULT_NUM_PROBES: u8 = 7;
const MIN_BITSET_BITS: usize = 64;
const BITS_PER_BYTE: usize = 8;
const PROBE_DELTA_ROTATION: u32 = 15;
const FILTER_HEADER_SIZE: usize = 8;

/// Builder for a prefix Bloom filter block.
pub struct BloomFilterBuilder {
    num_probes: u8,
    bits_per_key: usize,
    hashes: Vec<u32>,
}

impl BloomFilterBuilder {
    /// Create a builder with 10 bits per key and 7 probes.
    pub fn new() -> Self {
        Self {
            bits_per_key: DEFAULT_BITS_PER_KEY,
            num_probes: DEFAULT_NUM_PROBES,
            hashes: Vec::new(),
        }
    }

    /// Add one key prefix to the filter.
    pub fn add_key(&mut self, key: &[u8]) {
        self.hashes.push(compute_crc32c(key));
    }

    /// Serialize the filter block.
    pub fn build(self) -> Vec<u8> {
        let bitset_bits = if self.hashes.is_empty() {
            MIN_BITSET_BITS
        } else {
            (self.hashes.len() * self.bits_per_key).max(MIN_BITSET_BITS)
        };
        let bitset_bytes = bitset_bits.div_ceil(BITS_PER_BYTE);
        let mut bitset = vec![0u8; bitset_bytes];

        for hash in self.hashes {
            let delta = hash.rotate_left(PROBE_DELTA_ROTATION);
            let mut probe_hash = hash;
            for _ in 0..self.num_probes {
                let bit_position = (probe_hash as usize) % (bitset_bytes * BITS_PER_BYTE);
                bitset[bit_position / BITS_PER_BYTE] |= 1 << (bit_position % BITS_PER_BYTE);
                probe_hash = probe_hash.wrapping_add(delta);
            }
        }

        let mut encoded = Vec::with_capacity(FILTER_HEADER_SIZE + bitset_bytes);
        encoded.push(self.num_probes);
        encoded.push(self.bits_per_key as u8);
        encoded.extend_from_slice(&0u16.to_le_bytes());
        encoded.extend_from_slice(&(bitset_bytes as u32).to_le_bytes());
        encoded.extend_from_slice(&bitset);
        encoded
    }
}

/// Reader for a prefix Bloom filter block.
pub struct BloomFilterReader {
    num_probes: u8,
    bitset: Bytes,
}

impl BloomFilterReader {
    /// Parse a filter block.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::Corruption`] on a short or invalid block.
    pub fn new(data: Bytes) -> Result<Self> {
        if data.len() < FILTER_HEADER_SIZE {
            return Err(StorkvError::Corruption("Filter block too short".into()));
        }
        let num_probes = data[0];
        let bitset_length = u32::from_le_bytes(
            data[4..8]
                .try_into()
                .map_err(|_| StorkvError::Corruption("Invalid filter length field".into()))?,
        ) as usize;
        let bitset_end = FILTER_HEADER_SIZE
            .checked_add(bitset_length)
            .ok_or_else(|| StorkvError::Corruption("Filter bitset length overflow".into()))?;
        if data.len() < bitset_end {
            return Err(StorkvError::Corruption("Truncated filter bitset".into()));
        }
        let bitset = data.slice(FILTER_HEADER_SIZE..bitset_end);
        Ok(Self { num_probes, bitset })
    }

    /// Return false only when the filter proves that the key is absent.
    pub fn may_contain(&self, key: &[u8]) -> bool {
        if self.bitset.is_empty() {
            return true;
        }
        let total_bits = self.bitset.len() * BITS_PER_BYTE;
        let hash = compute_crc32c(key);
        let delta = hash.rotate_left(PROBE_DELTA_ROTATION);
        let mut probe_hash = hash;
        for _ in 0..self.num_probes {
            let bit_position = (probe_hash as usize) % total_bits;
            if (self.bitset[bit_position / BITS_PER_BYTE] & (1 << (bit_position % BITS_PER_BYTE)))
                == 0
            {
                return false;
            }
            probe_hash = probe_hash.wrapping_add(delta);
        }
        true
    }
}
