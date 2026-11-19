//! Decode the committed v0.1.0 binary fixtures with the current codecs.
//!
//! The fixtures are small, deterministic files that freeze the on-disk and
//! wire layout. A decoder change that breaks backward compatibility fails here
//! before it reaches a release.

use std::path::PathBuf;
use std::sync::Arc;

use bytes::{Buf, Bytes, BytesMut};
use storkv_core::format::compute_crc32c;
use storkv_core::manifest::{EditRecord, VersionEdit};
use storkv_core::sstable::{FOOTER_SIZE, SstReader, decompress_and_verify_block};
use storkv_core::vfs::{FileSystem, PosixFs};
use storkv_core::wal::{WAL_FRAME_HEADER_SIZE, WAL_HEADER_SIZE, WAL_MAGIC, WalBatchFrame, WalOp};
use storkv_proto::{Message, MessagePayload, Request, Response};

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/format/fixtures");

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(FIXTURE_DIR).join(name)).expect("read golden fixture")
}

fn assert_header(bytes: &[u8], magic: &[u8; 8], size: usize) {
    assert!(bytes.len() >= size);
    assert_eq!(&bytes[0..8], magic);
    let mut header = &bytes[8..size];
    assert_eq!(header.get_u16_le(), 1, "format version");
    assert_eq!(header.get_u16_le(), 0, "header flags");
    assert_eq!(header.get_u32_le(), 1_700_000_000, "creation seconds");
}

#[test]
fn v0_1_0_wal_fixture_decodes_all_record_kinds() {
    let bytes = fixture("v0_1_0.wal");
    assert_header(&bytes, WAL_MAGIC, WAL_HEADER_SIZE);

    let frame_bytes = &bytes[WAL_HEADER_SIZE..];
    let (frame, consumed) = WalBatchFrame::decode(frame_bytes).expect("decode WAL frame");
    assert_eq!(consumed, frame_bytes.len(), "no trailing WAL bytes");
    assert_eq!(frame.batch_seq, 42);
    assert_eq!(
        frame.ops,
        vec![
            WalOp::Put {
                scope_id: 1,
                expires_at_ms: 0,
                key: Bytes::from_static(b"user:1"),
                val: Bytes::from_static(b"alice"),
            },
            WalOp::Put {
                scope_id: 1,
                expires_at_ms: 1_893_456_000_000,
                key: Bytes::from_static(b"session:1"),
                val: Bytes::from_static(b"token"),
            },
            WalOp::Delete {
                scope_id: 1,
                key: Bytes::from_static(b"user:old"),
            },
            WalOp::Merge {
                scope_id: 1,
                key: Bytes::from_static(b"counter:1"),
                val: Bytes::from_static(b"7"),
            },
        ]
    );

    let stored_crc = u32::from_le_bytes(frame_bytes[0..4].try_into().expect("CRC field"));
    assert_eq!(stored_crc, compute_crc32c(&frame_bytes[4..consumed]));
    assert!(frame_bytes.len() > WAL_FRAME_HEADER_SIZE);
}

#[test]
fn v0_1_0_sst_fixture_decodes_blocks_and_footer() {
    let path = PathBuf::from(FIXTURE_DIR).join("v0_1_0.sst");
    let posix = Arc::new(PosixFs);
    let mut file = posix.open_file(&path).expect("open SST fixture");
    let reader = SstReader::open(&mut file).expect("parse SST fixture");

    assert_eq!(reader.meta.scope_id, 10);
    assert_eq!(reader.meta.level, 1);
    assert_eq!(reader.meta.entry_count, 3);
    assert_eq!(reader.meta.tombstone_count, 0);
    assert_eq!(reader.meta.min_seq, 100);
    assert_eq!(reader.meta.max_seq, 102);

    let entries = reader
        .read_all_entries(&mut file)
        .expect("read data entries");
    let keys: Vec<Bytes> = entries
        .iter()
        .map(|(internal_key, _)| {
            storkv_core::format::InternalKey::decode(internal_key.clone())
                .expect("decode internal key")
                .user_key
        })
        .collect();
    assert_eq!(
        keys,
        vec![
            Bytes::from_static(b"user:alpha"),
            Bytes::from_static(b"user:bravo"),
            Bytes::from_static(b"user:charlie"),
        ]
    );
    assert_eq!(entries[0].1, Bytes::from_static(b"one"));
    assert_eq!(entries[2].1, Bytes::from_static(b"three"));

    assert!(reader.filter.may_contain(b"user"));
    let raw = SstReader::read_raw_block(&mut file, reader.footer.meta_handle)
        .expect("read metadata block");
    let decoded = decompress_and_verify_block(&raw).expect("verify metadata block");
    assert_eq!(&decoded[..4], &10u32.to_le_bytes());

    let bytes = fixture("v0_1_0.sst");
    assert_eq!(&bytes[bytes.len() - 8..], b"STORKSST");
    assert!(bytes.len() > FOOTER_SIZE);
}

#[test]
fn v0_1_0_manifest_fixture_replays_scope_and_sstable_edits() {
    let bytes = fixture("v0_1_0.mnf");
    assert_header(&bytes, b"STORKMNF", 16);

    let (edit, consumed) = VersionEdit::decode(&bytes[16..]).expect("decode manifest edit");
    assert_eq!(consumed, bytes.len() - 16, "no trailing manifest bytes");
    assert!(matches!(edit.edits[0], EditRecord::SetNextSequence(1000)));
    assert!(matches!(edit.edits[1], EditRecord::SetWalNumber(42)));
    match &edit.edits[2] {
        EditRecord::CreateScope {
            scope_id,
            default_ttl_ms,
            compaction_style,
            name,
            ..
        } => {
            assert_eq!(*scope_id, 10);
            assert_eq!(*default_ttl_ms, 60_000);
            assert_eq!(*compaction_style, 1);
            assert_eq!(name, "golden_scope");
        }
        other => panic!("unexpected scope edit: {other:?}"),
    }
    match &edit.edits[3] {
        EditRecord::AddSstFile(meta) => {
            assert_eq!(meta.scope_id, 10);
            assert_eq!(meta.level, 1);
            assert_eq!(meta.file_num, 99);
            assert_eq!(meta.min_key, Bytes::from_static(b"k_start"));
            assert_eq!(meta.max_key, Bytes::from_static(b"k_end"));
        }
        other => panic!("unexpected SST edit: {other:?}"),
    }
    assert!(matches!(
        edit.edits[4],
        EditRecord::PinSnapshot { seq: 500, .. }
    ));
}

#[test]
fn v0_1_0_proto_fixture_decodes_request_and_response() {
    let bytes = fixture("v0_1_0.prt");
    let mut buffer = BytesMut::from(&bytes[..]);

    let request = Message::decode(&mut buffer)
        .expect("decode request")
        .expect("request frame");
    assert_eq!(request.request_id, 7);
    assert_eq!(
        request.payload,
        MessagePayload::Request(Request::Put {
            scope_id: 1,
            key: Bytes::from_static(b"user:1"),
            val: Bytes::from_static(b"alice"),
        })
    );

    let response = Message::decode(&mut buffer)
        .expect("decode response")
        .expect("response frame");
    assert_eq!(response.request_id, 7);
    assert_eq!(
        response.payload,
        MessagePayload::Response(Response::Value(Some(Bytes::from_static(b"alice"))))
    );
    assert!(buffer.is_empty(), "no trailing wire bytes");
}
