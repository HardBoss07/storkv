//! Condition wire type for atomic conditional writes.

use bytes::Bytes;

/// Condition attached to a [`crate::Request::PutIf`] operation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WirePutCondition {
    /// Write only when the key is absent or expired.
    Absent,
    /// Write only when the key is present and not expired.
    Present,
    /// Write only when the visible version has this sequence number.
    ExpectedSeq(u64),
    /// Write only when the visible value equals these bytes.
    ExpectedValue(Bytes),
}

/// Write one condition tag and its payload to `destination`.
pub(crate) fn encode_put_condition(
    destination: &mut bytes::BytesMut,
    condition: &WirePutCondition,
) {
    use bytes::BufMut;

    match condition {
        WirePutCondition::Absent => destination.put_u8(0),
        WirePutCondition::Present => destination.put_u8(1),
        WirePutCondition::ExpectedSeq(seq) => {
            destination.put_u8(2);
            destination.put_u64_le(*seq);
        }
        WirePutCondition::ExpectedValue(value) => {
            destination.put_u8(3);
            crate::frame::put_bytes(destination, value);
        }
    }
}
