use bytes::{BufMut, Bytes, BytesMut};

pub const MAX_KEY_LEN: usize = 65_535;
pub const MAX_VAL_LEN: usize = 268_435_456; // 256 MiB

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ValueType {
    Value = 0x01,
    Tombstone = 0x02,
    MergeOperand = 0x03,
    RangeTombstone = 0x04,
}

impl ValueType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(ValueType::Value),
            0x02 => Some(ValueType::Tombstone),
            0x03 => Some(ValueType::MergeOperand),
            0x04 => Some(ValueType::RangeTombstone),
            _ => None,
        }
    }
}

/// Internal Key encoding:
/// +-----------------------+------------------------+-------------------------+
/// | user_key ([u8; N])    | inverted_tag (u64 BE)  | expires_at_ms (u64 LE)  |
/// +-----------------------+------------------------+-------------------------+
/// |<------ N bytes ------>|<------- 8 bytes ------>|<------- 8 bytes ------->|
///
/// inverted_tag = !((seq << 8) | (value_type as u64)) serialized in Big-Endian.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InternalKey {
    pub user_key: Bytes,
    pub seq: u64,
    pub value_type: ValueType,
    pub expires_at_ms: u64,
}

impl Ord for InternalKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // User key ascending
        match self.user_key.cmp(&other.user_key) {
            std::cmp::Ordering::Equal => {
                // Sequence number descending (newest first)
                match other.seq.cmp(&self.seq) {
                    std::cmp::Ordering::Equal => {
                        (self.value_type as u8).cmp(&(other.value_type as u8))
                    }
                    ord => ord,
                }
            }
            ord => ord,
        }
    }
}

impl PartialOrd for InternalKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl InternalKey {
    pub fn new(user_key: Bytes, seq: u64, value_type: ValueType, expires_at_ms: u64) -> Self {
        Self {
            user_key,
            seq,
            value_type,
            expires_at_ms,
        }
    }

    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(self.user_key.len() + 16);
        buf.extend_from_slice(&self.user_key);
        let tag = !((self.seq << 8) | (self.value_type as u64));
        buf.put_u64(tag); // Big-Endian
        buf.put_u64_le(self.expires_at_ms); // Little-Endian
        buf.freeze()
    }

    pub fn decode(bytes: Bytes) -> Option<Self> {
        if bytes.len() < 16 {
            return None;
        }
        let user_key_len = bytes.len() - 16;
        let user_key = bytes.slice(0..user_key_len);
        let tag_bytes: [u8; 8] = bytes[user_key_len..user_key_len + 8].try_into().ok()?;
        let exp_bytes: [u8; 8] = bytes[user_key_len + 8..].try_into().ok()?;

        let inverted_tag = u64::from_be_bytes(tag_bytes);
        let tag = !inverted_tag;
        let seq = tag >> 8;
        let value_type = ValueType::from_u8((tag & 0xFF) as u8)?;
        let expires_at_ms = u64::from_le_bytes(exp_bytes);

        Some(Self {
            user_key,
            seq,
            value_type,
            expires_at_ms,
        })
    }
}

/// Calculate Castagnoli CRC32C checksum
pub fn compute_crc32c(data: &[u8]) -> u32 {
    crc32c::crc32c(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_internal_key_encoding_and_ordering() {
        let k1 = InternalKey::new(Bytes::from_static(b"apple"), 100, ValueType::Value, 0);
        let k2 = InternalKey::new(Bytes::from_static(b"apple"), 200, ValueType::Value, 0);
        let k3 = InternalKey::new(Bytes::from_static(b"banana"), 50, ValueType::Value, 0);

        let enc1 = k1.encode();
        let enc2 = k2.encode();
        let enc3 = k3.encode();

        // user_key "apple" < "banana"
        assert!(enc1 < enc3);
        assert!(enc2 < enc3);

        // For same user_key "apple", seq 200 must sort BEFORE seq 100
        assert!(
            enc2 < enc1,
            "Higher sequence number must sort smaller (descending)"
        );

        let dec1 = InternalKey::decode(enc1).unwrap();
        assert_eq!(dec1, k1);

        let dec2 = InternalKey::decode(enc2).unwrap();
        assert_eq!(dec2, k2);
    }

    #[test]
    fn test_crc32c() {
        let data = b"STORKV_TEST_PAYLOAD";
        let crc = compute_crc32c(data);
        assert_eq!(crc, crc32c::crc32c(data));
    }
}
