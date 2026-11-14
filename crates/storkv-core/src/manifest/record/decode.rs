use bytes::{Buf, Bytes};

use super::types::{EditRecord, SstFileMeta};
use crate::error::{Result, StorKvError};

pub(super) fn decode_edit_record(source: &mut impl Buf) -> Result<EditRecord> {
    let tag = read_tag(source)?;
    match tag {
        0x01 => Ok(EditRecord::SetNextSequence(read_u64(
            source,
            "SetNextSequence",
        )?)),
        0x02 => Ok(EditRecord::SetWalNumber(read_u64(source, "SetWalNumber")?)),
        0x03 => decode_create_scope(source),
        0x04 => decode_drop_scope(source),
        0x05 => decode_add_sst_file(source),
        0x06 => decode_delete_sst_file(source),
        0x07 => decode_pin_snapshot(source),
        0x08 => decode_unpin_snapshot(source),
        _ => Err(StorKvError::Corruption(format!(
            "Unknown EditRecord tag: 0x{:02x}",
            tag
        ))),
    }
}

fn read_tag(source: &mut impl Buf) -> Result<u8> {
    if source.remaining() < 1 {
        return Err(StorKvError::Corruption("Incomplete EditRecord tag".into()));
    }
    Ok(source.get_u8())
}

fn read_u64(source: &mut impl Buf, field: &str) -> Result<u64> {
    if source.remaining() < 8 {
        return Err(StorKvError::Corruption(format!("Incomplete {}", field)));
    }
    Ok(source.get_u64_le())
}

fn read_name(source: &mut impl Buf, field: &str) -> Result<String> {
    if source.remaining() < 2 {
        return Err(StorKvError::Corruption(format!("Incomplete {}", field)));
    }
    let name_length = source.get_u16_le() as usize;
    if source.remaining() < name_length {
        return Err(StorKvError::Corruption(format!(
            "Incomplete {} name",
            field
        )));
    }
    let mut name_bytes = vec![0u8; name_length];
    source.copy_to_slice(&mut name_bytes);
    String::from_utf8(name_bytes).map_err(|error| StorKvError::Corruption(error.to_string()))
}

fn decode_create_scope(source: &mut impl Buf) -> Result<EditRecord> {
    if source.remaining() < 4 + 1 + 4 + 1 + 8 + 1 + 2 {
        return Err(StorKvError::Corruption("Incomplete CreateScope".into()));
    }
    let scope_id = source.get_u32_le();
    let durability = source.get_u8();
    let sync_ms = source.get_u32_le();
    let retention = source.get_u8();
    let default_ttl_ms = source.get_u64_le();
    let compaction_style = source.get_u8();
    let name = read_name(source, "CreateScope")?;
    Ok(EditRecord::CreateScope {
        scope_id,
        durability,
        sync_ms,
        retention,
        default_ttl_ms,
        compaction_style,
        name,
    })
}

fn decode_drop_scope(source: &mut impl Buf) -> Result<EditRecord> {
    if source.remaining() < 4 + 8 {
        return Err(StorKvError::Corruption("Incomplete DropScope".into()));
    }
    Ok(EditRecord::DropScope {
        scope_id: source.get_u32_le(),
        drop_seq: source.get_u64_le(),
    })
}

fn decode_add_sst_file(source: &mut impl Buf) -> Result<EditRecord> {
    if source.remaining() < 4 + 1 + 8 + 8 + 8 + 8 + 8 + 2 {
        return Err(StorKvError::Corruption("Incomplete AddSstFile".into()));
    }
    let scope_id = source.get_u32_le();
    let level = source.get_u8();
    let file_num = source.get_u64_le();
    let file_size = source.get_u64_le();
    let min_seq = source.get_u64_le();
    let max_seq = source.get_u64_le();
    let max_expires_at_ms = source.get_u64_le();
    let min_key = read_bytes_field(source, "AddSstFile min_key")?;
    let max_key = read_bytes_field(source, "AddSstFile max_key")?;

    Ok(EditRecord::AddSstFile(SstFileMeta {
        scope_id,
        level,
        file_num,
        file_size,
        min_seq,
        max_seq,
        max_expires_at_ms,
        min_key: Bytes::from(min_key),
        max_key: Bytes::from(max_key),
    }))
}

fn read_bytes_field(source: &mut impl Buf, field: &str) -> Result<Vec<u8>> {
    if source.remaining() < 2 {
        return Err(StorKvError::Corruption(format!("Incomplete {}", field)));
    }
    let field_length = source.get_u16_le() as usize;
    if source.remaining() < field_length {
        return Err(StorKvError::Corruption(format!("Incomplete {}", field)));
    }
    let mut field_bytes = vec![0u8; field_length];
    source.copy_to_slice(&mut field_bytes);
    Ok(field_bytes)
}

fn decode_delete_sst_file(source: &mut impl Buf) -> Result<EditRecord> {
    if source.remaining() < 4 + 1 + 8 {
        return Err(StorKvError::Corruption("Incomplete DeleteSstFile".into()));
    }
    Ok(EditRecord::DeleteSstFile {
        scope_id: source.get_u32_le(),
        level: source.get_u8(),
        file_num: source.get_u64_le(),
    })
}

fn decode_pin_snapshot(source: &mut impl Buf) -> Result<EditRecord> {
    if source.remaining() < 8 + 8 + 2 {
        return Err(StorKvError::Corruption("Incomplete PinSnapshot".into()));
    }
    let seq = source.get_u64_le();
    let created_at_ms = source.get_u64_le();
    let name = read_name(source, "PinSnapshot")?;
    Ok(EditRecord::PinSnapshot {
        seq,
        created_at_ms,
        name,
    })
}

fn decode_unpin_snapshot(source: &mut impl Buf) -> Result<EditRecord> {
    let name = read_name(source, "UnpinSnapshot")?;
    Ok(EditRecord::UnpinSnapshot { name })
}
