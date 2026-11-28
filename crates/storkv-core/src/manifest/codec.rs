//! Manifest binary codec.
use bytes::{Buf, BufMut, BytesMut};

use crate::error::{Result, StorkvError};
use crate::format::compute_crc32c;

use super::record::EditRecord;

const EDIT_COUNT_SIZE: usize = 2;
const FRAME_HEADER_SIZE: usize = 8;

/// A VersionEdit frame holding multiple EditRecords.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionEdit {
    /// Records in the frame.
    pub edits: Vec<EditRecord>,
}

impl VersionEdit {
    /// Create an empty frame.
    pub fn new() -> Self {
        Self { edits: Vec::new() }
    }

    /// Append one record to the frame.
    pub fn add(&mut self, edit: EditRecord) {
        self.edits.push(edit);
    }

    /// Encode the frame with a CRC32C header.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = BytesMut::new();
        body.put_u16_le(self.edits.len() as u16);
        for edit in &self.edits {
            edit.encode(&mut body);
        }

        let mut payload_with_length = BytesMut::with_capacity(4 + body.len());
        payload_with_length.put_u32_le(body.len() as u32);
        payload_with_length.extend_from_slice(&body);

        let checksum = compute_crc32c(&payload_with_length);
        let mut frame = Vec::with_capacity(FRAME_HEADER_SIZE + payload_with_length.len());
        frame.extend_from_slice(&checksum.to_le_bytes());
        frame.extend_from_slice(&payload_with_length);
        frame
    }

    /// Decode one frame and return it with the number of consumed bytes.
    ///
    /// # Errors
    ///
    /// Returns `StorkvError::Corruption` on a short frame, checksum mismatch,
    /// or invalid record.
    pub fn decode(source: &[u8]) -> Result<(Self, usize)> {
        let (checksum, payload_length) = decode_frame_header(source)?;
        let total_frame_len = FRAME_HEADER_SIZE
            .checked_add(payload_length)
            .ok_or_else(|| StorkvError::Corruption("VersionEdit frame length overflow".into()))?;
        if source.len() < total_frame_len {
            return Err(StorkvError::Corruption(
                "Truncated VersionEdit frame payload".into(),
            ));
        }

        verify_checksum(checksum, &source[FRAME_HEADER_SIZE - 4..total_frame_len])?;

        let mut payload = &source[FRAME_HEADER_SIZE..total_frame_len];
        if payload.remaining() < EDIT_COUNT_SIZE {
            return Err(StorkvError::Corruption("Incomplete edit_count".into()));
        }
        let edit_count = payload.get_u16_le() as usize;
        let mut edits = Vec::with_capacity(edit_count);
        for _ in 0..edit_count {
            edits.push(EditRecord::decode(&mut payload)?);
        }

        Ok((VersionEdit { edits }, total_frame_len))
    }
}

fn decode_frame_header(source: &[u8]) -> Result<(u32, usize)> {
    if source.len() < FRAME_HEADER_SIZE {
        return Err(StorkvError::Corruption(
            "Truncated VersionEdit frame header".into(),
        ));
    }
    let checksum = u32::from_le_bytes(
        source[0..4]
            .try_into()
            .map_err(|_| StorkvError::Corruption("Invalid VersionEdit CRC field".into()))?,
    );
    let payload_length = u32::from_le_bytes(
        source[4..8]
            .try_into()
            .map_err(|_| StorkvError::Corruption("Invalid VersionEdit length field".into()))?,
    ) as usize;
    Ok((checksum, payload_length))
}

fn verify_checksum(expected: u32, payload_with_length: &[u8]) -> Result<()> {
    let computed = compute_crc32c(payload_with_length);
    if expected != computed {
        return Err(StorkvError::Corruption(format!(
            "VersionEdit CRC mismatch: expected 0x{:08x}, computed 0x{:08x}",
            expected, computed
        )));
    }
    Ok(())
}
