use bytes::Bytes;
use std::fs;
use std::io::Write;
use std::sync::Arc;
use storkv_core::format::{InternalKey, ValueType};
use storkv_core::sstable::{
    BLOCK_TRAILER_SIZE, CompressionType, FOOTER_SIZE, SST_MAGIC, SstReader, SstWriter,
    compress_and_frame_block, decompress_and_verify_block,
};
use storkv_core::vfs::{FileSystem, PosixFs};

#[test]
fn test_sstable_binary_structure_and_blocks() {
    let temp = std::env::temp_dir().join(format!("storkv_sst_layout_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();

    let fs_posix = Arc::new(PosixFs);
    let sst_path = temp.join("0000000000000001.sst");

    {
        let mut file = fs_posix.create_file(&sst_path).unwrap();
        let mut writer = SstWriter::new(&mut file, 1, 0, CompressionType::None, Some(4));

        for index in 0..50 {
            let user_key = format!("user:{:05}", index).into_bytes();
            let internal_key = InternalKey::new(
                Bytes::copy_from_slice(&user_key),
                100 + index as u64,
                ValueType::Value,
                0,
            );
            let encoded_key = internal_key.encode();
            let value = format!("val:{:05}", index).into_bytes();
            writer
                .add(
                    &encoded_key,
                    &user_key,
                    100 + index as u64,
                    0,
                    false,
                    &value,
                )
                .unwrap();
        }

        let (_footer, meta, _, _) = writer.finish().unwrap();
        assert_eq!(meta.entry_count, 50);
        assert_eq!(meta.scope_id, 1);
        assert_eq!(meta.level, 0);
    }

    let sst_bytes = fs::read(&sst_path).unwrap();
    assert!(sst_bytes.len() >= FOOTER_SIZE);

    let footer_start = sst_bytes.len() - FOOTER_SIZE;
    let footer_bytes = &sst_bytes[footer_start..];
    assert_eq!(&footer_bytes[48..56], SST_MAGIC);

    let meta_offset = u64::from_le_bytes(footer_bytes[0..8].try_into().unwrap());
    let meta_size = u64::from_le_bytes(footer_bytes[8..16].try_into().unwrap());
    assert!(meta_offset < footer_start as u64);
    assert!(meta_size > 0);

    let raw_payload = b"test_block_data_for_compression_and_trailer";
    let compressed_none = compress_and_frame_block(raw_payload, CompressionType::None).unwrap();
    assert_eq!(
        compressed_none[compressed_none.len() - BLOCK_TRAILER_SIZE],
        CompressionType::None as u8
    );
    let decompressed = decompress_and_verify_block(&compressed_none).unwrap();
    assert_eq!(decompressed, raw_payload);

    let compressed_lz4 = compress_and_frame_block(raw_payload, CompressionType::Lz4).unwrap();
    assert_eq!(
        compressed_lz4[compressed_lz4.len() - BLOCK_TRAILER_SIZE],
        CompressionType::Lz4 as u8
    );
    let decompressed_lz4 = decompress_and_verify_block(&compressed_lz4).unwrap();
    assert_eq!(decompressed_lz4, raw_payload);

    let compressed_zstd = compress_and_frame_block(raw_payload, CompressionType::Zstd).unwrap();
    assert_eq!(
        compressed_zstd[compressed_zstd.len() - BLOCK_TRAILER_SIZE],
        CompressionType::Zstd as u8
    );
    let decompressed_zstd = decompress_and_verify_block(&compressed_zstd).unwrap();
    assert_eq!(decompressed_zstd, raw_payload);

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn test_multi_block_sstable_with_hundreds_of_restarts() {
    let temp = std::env::temp_dir().join(format!("storkv_sst_multiblock_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();
    let fs_posix = Arc::new(PosixFs);
    let sst_path = temp.join("0000000000000001.sst");

    {
        let mut file = fs_posix.create_file(&sst_path).unwrap();
        let mut writer = SstWriter::new(&mut file, 1, 0, CompressionType::Lz4, Some(8));
        for index in 0..500u64 {
            let user_key = format!("key:{:05}", index).into_bytes();
            let internal_key = InternalKey::new(
                Bytes::copy_from_slice(&user_key),
                10_000 - index,
                ValueType::Value,
                0,
            );
            let value = vec![b'v'; 96];
            writer
                .add(
                    &internal_key.encode(),
                    &user_key,
                    10_000 - index,
                    0,
                    false,
                    &value,
                )
                .unwrap();
        }
        let (_footer, meta, _min, _max) = writer.finish().unwrap();
        assert_eq!(meta.entry_count, 500);
    }

    let mut file = fs_posix.open_file(&sst_path).unwrap();
    let reader = SstReader::open(&mut file).unwrap();
    assert!(
        reader.index.entries.len() > 1,
        "500 entries must span multiple data blocks"
    );
    assert!(
        reader.index.entries.len() >= 5,
        "large table must produce several index entries"
    );
    let entries = reader.read_all_entries(&mut file).unwrap();
    assert_eq!(entries.len(), 500);
    let first = InternalKey::decode(entries.first().unwrap().0.clone()).unwrap();
    let last = InternalKey::decode(entries.last().unwrap().0.clone()).unwrap();
    assert_eq!(first.user_key.as_ref(), b"key:00000");
    assert_eq!(last.user_key.as_ref(), b"key:00499");

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn test_sstable_single_bit_corruption_detected() {
    let temp = std::env::temp_dir().join(format!("storkv_sst_corrupt_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();
    let fs_posix = Arc::new(PosixFs);
    let path = temp.join("0000000000000001.sst");
    {
        let mut file = fs_posix.create_file(&path).unwrap();
        let mut writer = SstWriter::new(&mut file, 1, 0, CompressionType::None, None);
        for index in 0..100u64 {
            let user_key = format!("key:{:04}", index).into_bytes();
            let internal_key = InternalKey::new(
                Bytes::copy_from_slice(&user_key),
                index + 1,
                ValueType::Value,
                0,
            );
            writer
                .add(
                    &internal_key.encode(),
                    &user_key,
                    index + 1,
                    0,
                    false,
                    b"value",
                )
                .unwrap();
        }
        writer.finish().unwrap();
    }

    let original = fs::read(&path).unwrap();
    let offsets = [
        0usize,
        original.len() / 2,
        original.len() - 8,
        original.len() - FOOTER_SIZE + 48,
    ];
    for offset in offsets {
        let mut corrupted = original.clone();
        corrupted[offset] ^= 0x01;
        let corrupt_path = temp.join("corrupt.sst");
        {
            let mut file = fs_posix.create_file(&corrupt_path).unwrap();
            file.write_all(&corrupted).unwrap();
            file.sync_all().unwrap();
        }
        let mut file = fs_posix.open_file(&corrupt_path).unwrap();
        let result = SstReader::open(&mut file)
            .and_then(|reader| reader.read_all_entries(&mut file).map(|_| ()));
        assert!(
            result.is_err(),
            "SSTable corruption at offset {offset} must be detected"
        );
    }

    let _ = fs::remove_dir_all(&temp);
}
