//! Scan request encoding.
//!
//! The flags trailer is omitted when no flag is set. Older peers accept the
//! omitted form.

use bytes::{BufMut, BytesMut};

pub(super) fn encode_scan_flags(
    destination: &mut BytesMut,
    limit: Option<u32>,
    reverse: bool,
    keys_only: bool,
) {
    if limit.is_none() && !reverse && !keys_only {
        return;
    }
    let mut flags = 0u8;
    if limit.is_some() {
        flags |= 1;
    }
    if reverse {
        flags |= 2;
    }
    if keys_only {
        flags |= 4;
    }
    destination.put_u8(flags);
    if let Some(limit) = limit {
        destination.put_u32_le(limit);
    }
}
