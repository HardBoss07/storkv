use bytes::{Buf, BytesMut};

use super::decode::decode_edit_record;
use super::encode::encode_edit_record;
use super::types::EditRecord;
use crate::error::Result;

impl EditRecord {
    /// Append the record to a manifest buffer.
    pub fn encode(&self, destination: &mut BytesMut) {
        encode_edit_record(self, destination);
    }

    /// Decode one record from a manifest buffer.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorKvError::Corruption`] on a short or invalid record.
    pub fn decode(source: &mut impl Buf) -> Result<Self> {
        decode_edit_record(source)
    }
}
