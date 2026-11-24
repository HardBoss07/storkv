use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::error::{Result, StorkvError};
use crate::format::{MAX_VAL_LEN, compute_crc32c};

use super::record::{WalBatchFrame, WalOp};

/// Size of the fixed WAL frame header in bytes.
pub const WAL_FRAME_HEADER_SIZE: usize = 24;

/// Size of an unsigned 32-bit field.
const U32_SIZE: usize = 4;
const FRAME_LENGTH_AND_CRC_FIELDS: usize = 8;
const FIXED_BODY_SIZE: usize = 16;
const COMPRESSION_THRESHOLD: usize = 128;
const FLAG_COMPRESSED: u16 = 0x0001;
const MIN_ENCODED_OP_SIZE: usize = 17;

impl WalBatchFrame {
    /// Encode the batch with an optional LZ4 payload compression.
    ///
    /// # Errors
    ///
    /// Returns an error when an operation cannot be encoded.
    pub fn encode(&self, compress: bool) -> Result<Vec<u8>> {
        let payload = encode_ops(&self.ops)?;
        let (flags, frame_payload) = maybe_compress(payload, compress);

        let frame_length = FIXED_BODY_SIZE + frame_payload.len();
        let mut body_with_length = BytesMut::with_capacity(U32_SIZE + frame_length);
        body_with_length.put_u32_le(frame_length as u32);
        body_with_length.put_u64_le(self.batch_seq);
        body_with_length.put_u32_le(self.ops.len() as u32);
        body_with_length.put_u16_le(flags);
        body_with_length.put_u16_le(0);
        body_with_length.extend_from_slice(&frame_payload);

        let checksum = compute_crc32c(&body_with_length);
        let mut frame = Vec::with_capacity(U32_SIZE + body_with_length.len());
        frame.put_u32_le(checksum);
        frame.extend_from_slice(&body_with_length);
        Ok(frame)
    }

    /// Decode frame from byte buffer. Returns (frame, bytes_consumed)
    pub fn decode(source: &[u8]) -> Result<(Self, usize)> {
        let (expected_crc, total_frame_len) = decode_frame_header(source)?;
        verify_frame_crc(expected_crc, &source[4..total_frame_len])?;

        let mut body = &source[FRAME_LENGTH_AND_CRC_FIELDS..total_frame_len];
        let batch_seq = body.get_u64_le();
        let op_count = body.get_u32_le() as usize;
        let flags = body.get_u16_le();
        let _reserved = body.get_u16_le();
        let payload = decode_payload(body, flags)?;
        let ops = decode_ops(payload, op_count)?;

        Ok((WalBatchFrame { batch_seq, ops }, total_frame_len))
    }
}

fn encode_ops(ops: &[WalOp]) -> Result<BytesMut> {
    let mut payload = BytesMut::new();
    for op in ops {
        op.encode(&mut payload)?;
    }
    Ok(payload)
}

fn maybe_compress(payload: BytesMut, compress: bool) -> (u16, Bytes) {
    if compress && payload.len() > COMPRESSION_THRESHOLD {
        let mut compressed = BytesMut::new();
        compressed.put_u32_le(payload.len() as u32);
        compressed.extend_from_slice(&lz4_flex::compress_prepend_size(&payload));
        (FLAG_COMPRESSED, compressed.freeze())
    } else {
        (0, payload.freeze())
    }
}

fn decode_frame_header(source: &[u8]) -> Result<(u32, usize)> {
    if source.len() < WAL_FRAME_HEADER_SIZE {
        return Err(StorkvError::Corruption("Truncated WAL frame header".into()));
    }

    let expected_crc = u32::from_le_bytes(
        source[0..4]
            .try_into()
            .map_err(|_| StorkvError::Corruption("Invalid WAL CRC field".into()))?,
    );
    let frame_length = u32::from_le_bytes(
        source[4..8]
            .try_into()
            .map_err(|_| StorkvError::Corruption("Invalid WAL frame length field".into()))?,
    ) as usize;
    let total_frame_len = FRAME_LENGTH_AND_CRC_FIELDS
        .checked_add(frame_length)
        .ok_or_else(|| StorkvError::Corruption("WAL frame length overflow".into()))?;

    if source.len() < total_frame_len {
        return Err(StorkvError::Corruption(
            "Truncated WAL frame payload".into(),
        ));
    }
    if frame_length < FIXED_BODY_SIZE {
        return Err(StorkvError::Corruption(
            "WAL frame length smaller than fixed header".into(),
        ));
    }
    Ok((expected_crc, total_frame_len))
}

fn verify_frame_crc(expected_crc: u32, frame_body: &[u8]) -> Result<()> {
    let computed_crc = compute_crc32c(frame_body);
    if expected_crc != computed_crc {
        return Err(StorkvError::Corruption(format!(
            "WAL frame CRC mismatch: expected 0x{:08x}, got 0x{:08x}",
            expected_crc, computed_crc
        )));
    }
    Ok(())
}

fn decode_payload(mut body: &[u8], flags: u16) -> Result<Bytes> {
    if flags & FLAG_COMPRESSED != 0 {
        return decode_compressed_payload(&mut body);
    }
    Ok(Bytes::copy_from_slice(body))
}

fn decode_compressed_payload(body: &mut &[u8]) -> Result<Bytes> {
    if body.remaining() < U32_SIZE {
        return Err(StorkvError::Corruption(
            "Truncated compressed WAL payload".into(),
        ));
    }
    let uncompressed_len = body.get_u32_le() as usize;
    if uncompressed_len > MAX_VAL_LEN {
        return Err(StorkvError::Corruption(format!(
            "WAL payload length {uncompressed_len} exceeds the limit {MAX_VAL_LEN}"
        )));
    }
    let stored: &[u8] = body;
    if stored.len() < U32_SIZE {
        return Err(StorkvError::Corruption(
            "Truncated compressed WAL payload".into(),
        ));
    }
    let decompressed = lz4_flex::decompress(&stored[U32_SIZE..], uncompressed_len)
        .map_err(|error| StorkvError::Corruption(error.to_string()))?;
    Ok(Bytes::from(decompressed))
}

fn decode_ops(payload: Bytes, op_count: usize) -> Result<Vec<WalOp>> {
    if op_count > payload.len() / MIN_ENCODED_OP_SIZE {
        return Err(StorkvError::Corruption(
            "WAL op count exceeds available payload bytes".into(),
        ));
    }
    let mut remaining_payload = payload;
    let mut ops = Vec::with_capacity(op_count);
    for _ in 0..op_count {
        ops.push(WalOp::decode(&mut remaining_payload)?);
    }
    Ok(ops)
}
