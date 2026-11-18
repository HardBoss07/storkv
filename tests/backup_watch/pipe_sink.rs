use std::fs;
use std::path::Path;

use storkv_core::backup::{
    ARCHIVE_MAGIC, ARCHIVE_VERSION, BackupCompression, BackupConfig, CommandPipeSinkConfig,
};

use crate::support::backup_root;

#[test]
fn pipe_sink_streams_uncompressed_archive() {
    round_trip(BackupCompression::None);
}

#[test]
fn pipe_sink_streams_zstd_archive() {
    round_trip(BackupCompression::Zstd);
}

#[test]
fn pipe_sink_streams_lz4_archive() {
    round_trip(BackupCompression::Lz4);
}

fn round_trip(compression: BackupCompression) {
    let mut temp = crate::common::TempDb::new("pipe_sink");
    let db = temp.db();
    let scope = db
        .create_scope("pipe_scope", storkv_core::ScopeOptions::default())
        .unwrap();
    for index in 0..24 {
        db.put(
            &scope,
            format!("k{index:02}").as_bytes(),
            format!("v{index:02}").as_bytes(),
        )
        .unwrap();
    }

    let root = backup_root(&temp);
    let archive_path = temp.path.join("archive with spaces.bin");
    let config = capture_command(&archive_path, compression);
    db.start_auto_backup(BackupConfig::command(root.clone(), config).unwrap())
        .unwrap();
    db.trigger_backup(true).unwrap();
    db.stop_auto_backup().unwrap();
    temp.close();

    let archive = fs::read(&archive_path).unwrap();
    assert!(archive.len() > ARCHIVE_MAGIC.len() + 3);
    assert_eq!(&archive[0..8], ARCHIVE_MAGIC);
    assert_eq!(
        u16::from_le_bytes([archive[8], archive[9]]),
        ARCHIVE_VERSION
    );
    let tag = archive[10];
    let expected_tag = match compression {
        BackupCompression::None => 0,
        BackupCompression::Zstd => 1,
        BackupCompression::Lz4 => 2,
    };
    assert_eq!(tag, expected_tag);

    let payload = match tag {
        0 => archive[11..].to_vec(),
        1 => zstd::decode_all(&archive[11..]).unwrap(),
        2 => lz4_flex::decompress_size_prepended(&archive[11..]).unwrap(),
        other => panic!("unexpected compression tag {other}"),
    };

    let entries = parse_entries(&payload);
    assert!(
        entries
            .iter()
            .any(|(kind, path, _)| *kind == 3 && path == "BACKUP_MANIFEST.json"),
        "archive must include the manifest"
    );
    assert!(
        entries
            .iter()
            .any(|(kind, path, _)| *kind == 1 && path.ends_with(".sst")),
        "archive must include SSTables"
    );
    for (kind, path, bytes) in &entries {
        if *kind == 3 {
            let manifest: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            assert_eq!(manifest["format"], "STORKBACKUP");
            assert_eq!(manifest["compression"], compression.as_str());
        }
        assert!(!path.is_empty());
    }
}

fn parse_entries(payload: &[u8]) -> Vec<(u8, String, Vec<u8>)> {
    let mut entries = Vec::new();
    let mut offset = 0usize;
    loop {
        let kind = payload[offset];
        offset += 1;
        if kind == 0 {
            break;
        }
        let path_len = u32::from_be_bytes(payload[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        let path = String::from_utf8(payload[offset..offset + path_len].to_vec()).unwrap();
        offset += path_len;
        let data_len = u64::from_le_bytes(payload[offset..offset + 8].try_into().unwrap()) as usize;
        offset += 8;
        let checksum = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
        offset += 4;
        let data = payload[offset..offset + data_len].to_vec();
        offset += data_len;
        assert_eq!(checksum, storkv_core::format::compute_crc32c(&data));
        entries.push((kind, path, data));
    }
    entries
}

#[cfg(windows)]
fn capture_command(path: &Path, compression: BackupCompression) -> CommandPipeSinkConfig {
    let escaped = path.display().to_string().replace('\'', "''");
    let script = format!(
        "$s=[Console]::OpenStandardInput();$f=[IO.File]::Create('{escaped}');$s.CopyTo($f);$f.Close()"
    );
    CommandPipeSinkConfig::new("powershell")
        .with_args(vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            script,
        ])
        .with_compression(compression)
}

#[cfg(unix)]
fn capture_command(path: &Path, compression: BackupCompression) -> CommandPipeSinkConfig {
    let script = format!("cat > '{}'", path.display());
    CommandPipeSinkConfig::new("sh")
        .with_args(vec!["-c".into(), script])
        .with_compression(compression)
}
