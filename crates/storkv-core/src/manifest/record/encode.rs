use bytes::{BufMut, BytesMut};

use super::types::{
    EditRecord, SstFileMeta, StorageModeMode, TAG_ADD_RANGE_TOMBSTONE, TAG_ADD_SST_FILE,
    TAG_CREATE_SCOPE, TAG_CREATE_SCOPE_WITH_STORAGE, TAG_DELETE_SST_FILE, TAG_DROP_SCOPE,
    TAG_PIN_SNAPSHOT, TAG_SET_NEXT_SEQUENCE, TAG_SET_WAL_NUMBER, TAG_UNPIN_SNAPSHOT,
};

pub(super) fn encode_edit_record(record: &EditRecord, destination: &mut BytesMut) {
    match record {
        EditRecord::SetNextSequence(sequence_number) => {
            destination.put_u8(TAG_SET_NEXT_SEQUENCE);
            destination.put_u64_le(*sequence_number);
        }
        EditRecord::SetWalNumber(wal_number) => {
            destination.put_u8(TAG_SET_WAL_NUMBER);
            destination.put_u64_le(*wal_number);
        }
        EditRecord::CreateScope {
            scope_id,
            durability,
            sync_ms,
            retention,
            default_ttl_ms,
            compaction_style,
            storage_mode,
            name,
        } => {
            if *storage_mode == StorageModeMode::Disk as u8 {
                destination.put_u8(TAG_CREATE_SCOPE);
            } else {
                destination.put_u8(TAG_CREATE_SCOPE_WITH_STORAGE);
            }
            destination.put_u32_le(*scope_id);
            destination.put_u8(*durability);
            destination.put_u32_le(*sync_ms);
            destination.put_u8(*retention);
            destination.put_u64_le(*default_ttl_ms);
            destination.put_u8(*compaction_style);
            put_name(destination, name);
            if *storage_mode != StorageModeMode::Disk as u8 {
                destination.put_u8(*storage_mode);
            }
        }
        EditRecord::DropScope { scope_id, drop_seq } => {
            destination.put_u8(TAG_DROP_SCOPE);
            destination.put_u32_le(*scope_id);
            destination.put_u64_le(*drop_seq);
        }
        EditRecord::AddSstFile(metadata) => {
            destination.put_u8(TAG_ADD_SST_FILE);
            encode_sst_file_meta(destination, metadata);
        }
        EditRecord::DeleteSstFile {
            scope_id,
            level,
            file_num,
        } => {
            destination.put_u8(TAG_DELETE_SST_FILE);
            destination.put_u32_le(*scope_id);
            destination.put_u8(*level);
            destination.put_u64_le(*file_num);
        }
        EditRecord::PinSnapshot {
            seq,
            created_at_ms,
            name,
        } => {
            destination.put_u8(TAG_PIN_SNAPSHOT);
            destination.put_u64_le(*seq);
            destination.put_u64_le(*created_at_ms);
            put_name(destination, name);
        }
        EditRecord::UnpinSnapshot { name } => {
            destination.put_u8(TAG_UNPIN_SNAPSHOT);
            put_name(destination, name);
        }
        EditRecord::AddRangeTombstone {
            scope_id,
            seq,
            start_key,
            end_key,
        } => {
            destination.put_u8(TAG_ADD_RANGE_TOMBSTONE);
            destination.put_u32_le(*scope_id);
            destination.put_u64_le(*seq);
            put_bytes_field(destination, start_key);
            put_bytes_field(destination, end_key);
        }
    }
}

fn put_name(destination: &mut BytesMut, name: &str) {
    destination.put_u16_le(name.len() as u16);
    destination.extend_from_slice(name.as_bytes());
}

fn encode_sst_file_meta(destination: &mut BytesMut, metadata: &SstFileMeta) {
    destination.put_u32_le(metadata.scope_id);
    destination.put_u8(metadata.level);
    destination.put_u64_le(metadata.file_num);
    destination.put_u64_le(metadata.file_size);
    destination.put_u64_le(metadata.min_seq);
    destination.put_u64_le(metadata.max_seq);
    destination.put_u64_le(metadata.max_expires_at_ms);
    put_bytes_field(destination, &metadata.min_key);
    put_bytes_field(destination, &metadata.max_key);
}

fn put_bytes_field(destination: &mut BytesMut, field: &[u8]) {
    destination.put_u16_le(field.len() as u16);
    destination.extend_from_slice(field);
}
