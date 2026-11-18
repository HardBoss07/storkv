use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use storkv_core::manifest::ManifestManager;
use storkv_core::sstable::{
    BLOCK_TRAILER_SIZE, BlockHandle, BloomFilterReader, FOOTER_SIZE, Footer, IndexBlockReader,
    MetaBlock, decompress_and_verify_block,
};
use storkv_core::vfs::PosixFs;

use crate::CliRunner;

/// Result of an offline database verification run.
#[derive(Debug)]
pub struct DoctorReport {
    /// Number of SSTable files checked.
    pub sstables_checked: usize,
    /// Number of manifest files checked.
    pub manifests_checked: usize,
    /// Verification errors. An empty list means success.
    pub errors: Vec<String>,
}

impl CliRunner {
    /// Verify the manifest and every SSTable in an offline data directory.
    pub fn doctor<P: AsRef<Path>>(data_dir: P) -> DoctorReport {
        let root = data_dir.as_ref();
        let mut report = DoctorReport {
            sstables_checked: 0,
            manifests_checked: 0,
            errors: Vec::new(),
        };

        verify_current_manifest(root, &mut report);
        verify_scope_sstables(root, &mut report);
        report
    }
}

fn verify_current_manifest(root: &Path, report: &mut DoctorReport) {
    let current_path = root.join("CURRENT");
    if !current_path.exists() {
        report.errors.push("Missing CURRENT file".into());
        return;
    }

    let contents = match std::fs::read_to_string(&current_path) {
        Ok(contents) => contents,
        Err(error) => {
            report
                .errors
                .push(format!("Cannot read CURRENT file: {}", error));
            return;
        }
    };

    let manifest_name = contents.trim();
    let manifest_path = root.join(manifest_name);
    if !manifest_path.exists() {
        report.errors.push(format!(
            "Active manifest `{}` referenced by CURRENT not found",
            manifest_name
        ));
        return;
    }

    report.manifests_checked += 1;
    let file_system = Arc::new(PosixFs);
    if let Err(error) = ManifestManager::open(file_system, root) {
        report
            .errors
            .push(format!("Corrupt manifest file: {}", error));
    }
}

fn verify_scope_sstables(root: &Path, report: &mut DoctorReport) {
    let scopes_dir = root.join("scopes");
    if !scopes_dir.exists() {
        return;
    }
    let Ok(scope_entries) = std::fs::read_dir(&scopes_dir) else {
        return;
    };

    for scope_entry in scope_entries.flatten() {
        let scope_path = scope_entry.path();
        if !scope_path.is_dir() {
            continue;
        }
        let Ok(sst_entries) = std::fs::read_dir(&scope_path) else {
            continue;
        };
        for sst_entry in sst_entries.flatten() {
            verify_sst_entry(&sst_entry.path(), report);
        }
    }
}

fn verify_sst_entry(sst_path: &Path, report: &mut DoctorReport) {
    if sst_path
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("sst")
    {
        return;
    }
    report.sstables_checked += 1;
    if let Err(error) = verify_sstable(sst_path) {
        report.errors.push(format!(
            "SSTable `{}` failed verification: {}",
            sst_path.display(),
            error
        ));
    }
}

fn verify_sstable(path: &Path) -> Result<(), String> {
    let mut file = File::open(path).map_err(|error| format!("Open error: {}", error))?;
    let file_len = file
        .metadata()
        .map_err(|error| format!("Meta error: {}", error))?
        .len();
    if file_len < FOOTER_SIZE as u64 {
        return Err("File smaller than SSTable footer size".into());
    }

    let footer = read_footer(&mut file)?;
    let meta_bytes = verify_block_raw(&mut file, &footer.meta_handle, "Meta")?;
    MetaBlock::decode(&meta_bytes).map_err(|error| format!("Corrupt MetaBlock: {}", error))?;

    let filter_bytes = verify_block_raw(&mut file, &footer.filter_handle, "Filter")?;
    BloomFilterReader::new(bytes::Bytes::from(filter_bytes))
        .map_err(|error| format!("Corrupt BloomFilter: {}", error))?;

    let index_bytes = verify_block_raw(&mut file, &footer.index_handle, "Index")?;
    let index = IndexBlockReader::decode(&index_bytes)
        .map_err(|error| format!("Corrupt IndexBlock: {}", error))?;
    for (index_position, entry) in index.entries.iter().enumerate() {
        let block_name = format!("Data[{}]", index_position);
        verify_block_raw(&mut file, &entry.handle, &block_name)?;
    }

    Ok(())
}

fn read_footer(file: &mut File) -> Result<Footer, String> {
    file.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))
        .map_err(|error| format!("Seek error: {}", error))?;
    let mut footer_bytes = [0u8; FOOTER_SIZE];
    file.read_exact(&mut footer_bytes)
        .map_err(|error| format!("Read footer error: {}", error))?;
    Footer::decode(&footer_bytes).map_err(|error| format!("Invalid SSTable footer: {}", error))
}

fn verify_block_raw(file: &mut File, handle: &BlockHandle, name: &str) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(handle.offset))
        .map_err(|error| format!("Seek {} block error: {}", name, error))?;

    let full_length = handle.size as usize + BLOCK_TRAILER_SIZE;
    let mut block_bytes = vec![0u8; full_length];
    file.read_exact(&mut block_bytes)
        .map_err(|error| format!("Read {} payload error: {}", name, error))?;

    decompress_and_verify_block(&block_bytes)
        .map_err(|error| format!("{} block checksum failure: {}", name, error))
}
