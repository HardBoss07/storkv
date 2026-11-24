use bytes::{Buf, Bytes};

use super::types::{
    EditRecord, SstFileMeta, StorageModeMode, TAG_ADD_RANGE_TOMBSTONE, TAG_ADD_SST_FILE, TAG_CREATE_SCOPE,
    TAG_CREATE_SCOPE_WITH_STORAGE, TAG_DELETE_SST_FILE, TAG_DROP_SCOPE, TAG_PIN_SNAPSHOT,
    TAG_SET_NEXT_SEQUENCE, TAG_SET_WAL_NUMBER, TAG_UNPIN_SNAPSHOT,
};
use crate::error::{Result, StorkvError};

/// Size of a one-byte discriminant or flag field.
const U8_SIZE: usize = 1;
/// Size of a little-endian `u16` field.
const U16_SIZE: usize = 2;
/// Size of a little-endian `u32` field.
const U32_SIZE: usize = 4;
/// Size of a little-endian `u64` field.
const U64_SIZE: usize = 8;

pub(super) fn decode_edit_record(source: &mut impl Buf) -> Result<EditRecord> {
    let tag = read_tag(source)?;
    match tag {
        TAG_SET_NEXT_SEQUENCE => Ok(EditRecord::SetNextSequence(read_u64(
            source,
            "SetNextSequence",
        )?)),
        TAG_SET_WAL_NUMBER => Ok(EditRecord::SetWalNumber(read_u64(source, "SetWalNumber")?)),
        TAG_CREATE_SCOPE => decode_create_scope(source, StorageModeMode::Disk as u8),
        TAG_DROP_SCOPE => decode_drop_scope(source),
        TAG_ADD_SST_FILE => decode_add_sst_file(source),
        TAG_DELETE_SST_FILE => decode_delete_sst_file(source),
        TAG_PIN_SNAPSHOT => decode_pin_snapshot(source),
        TAG_UNPIN_SNAPSHOT => decode_unpin_snapshot(source),
        TAG_CREATE_SCOPE_WITH_STORAGE => decode_create_scope_with_storage(source),
        TAG_ADD_RANGE_TOMBSTONE => decode_add_range_tombstone(source),
        _ => Err(StorkvError::Corruption(format!(
            "Unknown EditRecord tag: 0x{:02x}",
            tag
        ))),
    }
}

fn read_tag(source: &mut impl Buf) -> Result<u8> {
    if source.remaining() < 1 {
        return Err(StorkvError::Corruption("Incomplete EditRecord tag".into()));
    }
    Ok(source.get_u8())
}

fn read_u64(source: &mut impl Buf, field: &str) -> Result<u64> {
    if source.remaining() < U64_SIZE {
        return Err(StorkvError::Corruption(format!("Incomplete {}", field)));
    }
    Ok(source.get_u64_le())
}

fn read_name(source: &mut impl Buf, field: &str) -> Result<String> {
    if source.remaining() < U16_SIZE {
        return Err(StorkvError::Corruption(format!("Incomplete {}", field)));
    }
    let name_length = source.get_u16_le() as usize;
    if source.remaining() < name_length {
        return Err(StorkvError::Corruption(format!(
            "Incomplete {} name",
            field
        )));
    }
    let mut name_bytes = vec![0u8; name_length];
    source.copy_to_slice(&mut name_bytes);
    String::from_utf8(name_bytes).map_err(|error| StorkvError::Corruption(error.to_string()))
}

fn decode_create_scope(source: &mut impl Buf, storage_mode: u8) -> Result<EditRecord> {
    if source.remaining() < U32_SIZE + U8_SIZE + U32_SIZE + U8_SIZE + U64_SIZE + U8_SIZE + U16_SIZE
    {
        return Err(StorkvError::Corruption("Incomplete CreateScope".into()));
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
        storage_mode,
        name,
    })
}

fn decode_create_scope_with_storage(source: &mut impl Buf) -> Result<EditRecord> {
    let mut record = decode_create_scope(source, StorageModeMode::Disk as u8)?;
    if source.remaining() < U8_SIZE {
        return Err(StorkvError::Corruption(
            "Incomplete CreateScope storage mode".into(),
        ));
    }
    let storage_mode = source.get_u8();
    if let EditRecord::CreateScope {
        storage_mode: slot, ..
    } = &mut record
    {
        *slot = storage_mode;
    }
    Ok(record)
}

fn decode_drop_scope(source: &mut impl Buf) -> Result<EditRecord> {
    if source.remaining() < U32_SIZE + U64_SIZE {
        return Err(StorkvError::Corruption("Incomplete DropScope".into()));
    }
    Ok(EditRecord::DropScope {
        scope_id: source.get_u32_le(),
        drop_seq: source.get_u64_le(),
    })
}

fn decode_add_sst_file(source: &mut impl Buf) -> Result<EditRecord> {
    if source.remaining()
        < U32_SIZE + U8_SIZE + U64_SIZE + U64_SIZE + U64_SIZE + U64_SIZE + U64_SIZE + U16_SIZE
    {
        return Err(StorkvError::Corruption("Incomplete AddSstFile".into()));
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
    if source.remaining() < U16_SIZE {
        return Err(StorkvError::Corruption(format!("Incomplete {}", field)));
    }
    let field_length = source.get_u16_le() as usize;
    if source.remaining() < field_length {
        return Err(StorkvError::Corruption(format!("Incomplete {}", field)));
    }
    let mut field_bytes = vec![0u8; field_length];
    source.copy_to_slice(&mut field_bytes);
    Ok(field_bytes)
}

fn decode_delete_sst_file(source: &mut impl Buf) -> Result<EditRecord> {
    if source.remaining() < U32_SIZE + U8_SIZE + U64_SIZE {
        return Err(StorkvError::Corruption("Incomplete DeleteSstFile".into()));
    }
    Ok(EditRecord::DeleteSstFile {
        scope_id: source.get_u32_le(),
        level: source.get_u8(),
        file_num: source.get_u64_le(),
    })
}

fn decode_pin_snapshot(source: &mut impl Buf) -> Result<EditRecord> {
    if source.remaining() < U64_SIZE + U64_SIZE + U16_SIZE {
        return Err(StorkvError::Corruption("Incomplete PinSnapshot".into()));
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

fn decode_add_range_tombstone(source: &mut impl Buf) -> Result<EditRecord> {
    if source.remaining() < U32_SIZE + U64_SIZE + U16_SIZE + U16_SIZE {
        return Err(StorkvError::Corruption(
            "Incomplete AddRangeTombstone".into(),
        ));
    }
    let scope_id = source.get_u32_le();
    let seq = source.get_u64_le();
    let start_key = read_bytes_field(source, "AddRangeTombstone start_key")?;
    let end_key = read_bytes_field(source, "AddRangeTombstone end_key")?;
    Ok(EditRecord::AddRangeTombstone {
        scope_id,
        seq,
        start_key: Bytes::from(start_key),
        end_key: Bytes::from(end_key),
    })
}
