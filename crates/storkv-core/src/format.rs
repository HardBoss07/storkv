use bytes::{BufMut, Bytes, BytesMut};

/// Maximum user key length in bytes.
pub const MAX_KEY_LEN: usize = 65_535;
/// Maximum value length in bytes (256 MiB).
pub const MAX_VAL_LEN: usize = 268_435_456; // 256 MiB

/// Kind of an internal key entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ValueType {
    /// A stored value.
    Value = 0x01,
    /// A point deletion marker.
    Tombstone = 0x02,
    /// A merge operand.
    MergeOperand = 0x03,
    /// A range deletion marker.
    RangeTombstone = 0x04,
}

impl ValueType {
    /// Decode a value type byte.
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
/// Internal key: user key, commit sequence, value type, and expiry time.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InternalKey {
    /// User key bytes.
    pub user_key: Bytes,
    /// Commit sequence number.
    pub seq: u64,
    /// Entry kind.
    pub value_type: ValueType,
    /// Absolute expiry time in milliseconds, or `0`.
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
    /// Build an internal key.
    pub fn new(user_key: Bytes, seq: u64, value_type: ValueType, expires_at_ms: u64) -> Self {
        Self {
            user_key,
            seq,
            value_type,
            expires_at_ms,
        }
    }

    /// Encode the key in sort order.
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(self.user_key.len() + 16);
        buf.extend_from_slice(&self.user_key);
        let tag = !((self.seq << 8) | (self.value_type as u64));
        buf.put_u64(tag); // Big-Endian
        buf.put_u64_le(self.expires_at_ms); // Little-Endian
        buf.freeze()
    }

    /// Decode a key. `None` means the input is too short or invalid.
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
