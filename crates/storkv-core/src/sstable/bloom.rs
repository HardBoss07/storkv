use crate::error::{Result, StorKvError};
use crate::format::compute_crc32c;
use bytes::Bytes;

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

        for hash in self.hashes {
            let delta = hash.rotate_left(15);
            let mut probe_hash = hash;
            for _ in 0..self.num_probes {
                let bit_position = (probe_hash as usize) % (bitset_bytes * 8);
                bitset[bit_position / 8] |= 1 << (bit_position % 8);
                probe_hash = probe_hash.wrapping_add(delta);
            }
        }

        let mut encoded = Vec::with_capacity(8 + bitset_bytes);
        encoded.push(self.num_probes);
        encoded.push(self.bits_per_key as u8);
        encoded.extend_from_slice(&0u16.to_le_bytes());
        encoded.extend_from_slice(&(bitset_bytes as u32).to_le_bytes());
        encoded.extend_from_slice(&bitset);
        encoded
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
        let bitset_length = u32::from_le_bytes(
            data[4..8]
                .try_into()
                .map_err(|_| StorKvError::Corruption("Invalid filter length field".into()))?,
        ) as usize;
        if data.len() < 8 + bitset_length {
            return Err(StorKvError::Corruption("Truncated filter bitset".into()));
        }
        let bitset = data.slice(8..8 + bitset_length);
        Ok(Self { num_probes, bitset })
    }

    pub fn may_contain(&self, key: &[u8]) -> bool {
        if self.bitset.is_empty() {
            return true;
        }
        let total_bits = self.bitset.len() * 8;
        let hash = compute_crc32c(key);
        let delta = hash.rotate_left(15);
        let mut probe_hash = hash;
        for _ in 0..self.num_probes {
            let bit_position = (probe_hash as usize) % total_bits;
            if (self.bitset[bit_position / 8] & (1 << (bit_position % 8))) == 0 {
                return false;
            }
            probe_hash = probe_hash.wrapping_add(delta);
        }
        true
    }
}
