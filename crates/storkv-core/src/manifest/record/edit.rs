use bytes::{Buf, BytesMut};

use super::decode::decode_edit_record;
use super::encode::encode_edit_record;
use super::types::EditRecord;
use crate::error::Result;

impl EditRecord {
    pub fn encode(&self, destination: &mut BytesMut) {
        encode_edit_record(self, destination);
    }

    pub fn decode(source: &mut impl Buf) -> Result<Self> {
        decode_edit_record(source)
    }
}
