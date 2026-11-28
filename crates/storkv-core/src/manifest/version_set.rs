//! Manifest version set state.
use std::collections::{BTreeMap, HashMap};

use bytes::Bytes;

use super::codec::VersionEdit;
use super::record::{
    CompactionStyleMode, DurabilityMode, EditRecord, RetentionMode, ScopeMeta, SnapshotMeta,
    SstFileMeta, StorageModeMode,
};

/// In-memory engine state reconstructed from MANIFEST
#[derive(Debug, Clone, Default)]
pub struct ManifestState {
    /// Next sequence number to allocate.
    pub next_seq: u64,
    /// Oldest WAL segment that recovery must replay.
    pub min_unflushed_wal_num: u64,
    /// Scope metadata keyed by identifier.
    pub scopes: HashMap<u32, ScopeMeta>,
    /// Scope identifiers keyed by name.
    pub scope_by_name: HashMap<String, u32>,
    /// SSTable metadata keyed by scope, level, and file number.
    pub sst_files: BTreeMap<(u32, u8, u64), SstFileMeta>,
    /// Pinned snapshots keyed by name.
    pub pinned_snapshots: HashMap<String, SnapshotMeta>,
    /// Range tombstones keyed by scope, sequence number, and start key.
    pub range_tombstones: BTreeMap<(u32, u64, Bytes), Bytes>,
}

impl ManifestState {
    /// Apply every record of one version edit.
    pub fn apply(&mut self, edit: &VersionEdit) {
        for record in &edit.edits {
            self.apply_record(record);
        }
    }

    fn apply_record(&mut self, record: &EditRecord) {
        match record {
            EditRecord::SetNextSequence(sequence_number) => {
                if *sequence_number > self.next_seq {
                    self.next_seq = *sequence_number;
                }
            }
            EditRecord::SetWalNumber(wal_number) => {
                self.min_unflushed_wal_num = *wal_number;
            }
            EditRecord::CreateScope { .. } => self.apply_create_scope(record),
            EditRecord::DropScope { scope_id, drop_seq } => {
                self.apply_drop_scope(*scope_id, *drop_seq);
            }
            EditRecord::AddSstFile(metadata) => {
                self.sst_files.insert(
                    (metadata.scope_id, metadata.level, metadata.file_num),
                    metadata.clone(),
                );
            }
            EditRecord::DeleteSstFile {
                scope_id,
                level,
                file_num,
            } => {
                self.sst_files.remove(&(*scope_id, *level, *file_num));
            }
            EditRecord::PinSnapshot {
                seq,
                created_at_ms,
                name,
            } => {
                self.pinned_snapshots.insert(
                    name.clone(),
                    SnapshotMeta {
                        name: name.clone(),
                        seq: *seq,
                        created_at_ms: *created_at_ms,
                    },
                );
            }
            EditRecord::UnpinSnapshot { name } => {
                self.pinned_snapshots.remove(name);
            }
            EditRecord::AddRangeTombstone {
                scope_id,
                seq,
                start_key,
                end_key,
            } => {
                self.range_tombstones
                    .insert((*scope_id, *seq, start_key.clone()), end_key.clone());
            }
        }
    }

    fn apply_create_scope(&mut self, record: &EditRecord) {
        let EditRecord::CreateScope {
            scope_id,
            durability,
            sync_ms,
            retention,
            default_ttl_ms,
            compaction_style,
            storage_mode,
            name,
        } = record
        else {
            return;
        };
        let metadata = ScopeMeta {
            scope_id: *scope_id,
            durability: durability_mode(*durability),
            sync_ms: *sync_ms,
            retention: retention_mode(*retention),
            default_ttl_ms: *default_ttl_ms,
            compaction_style: compaction_style_mode(*compaction_style),
            name: name.clone(),
            dropped: false,
            drop_seq: 0,
            storage_mode: storage_mode_mode(*storage_mode),
        };
        self.scopes.insert(*scope_id, metadata);
        self.scope_by_name.insert(name.clone(), *scope_id);
    }

    fn apply_drop_scope(&mut self, scope_id: u32, drop_seq: u64) {
        if let Some(scope) = self.scopes.get_mut(&scope_id) {
            scope.dropped = true;
            scope.drop_seq = drop_seq;
            self.scope_by_name.remove(&scope.name);
        }
    }
}

fn durability_mode(value: u8) -> DurabilityMode {
    if value == DurabilityMode::StrictFsync as u8 {
        DurabilityMode::StrictFsync
    } else {
        DurabilityMode::Buffered
    }
}

fn retention_mode(value: u8) -> RetentionMode {
    if value == RetentionMode::Permanent as u8 {
        RetentionMode::Permanent
    } else {
        RetentionMode::Ttl
    }
}

fn compaction_style_mode(value: u8) -> CompactionStyleMode {
    if value == CompactionStyleMode::Leveled as u8 {
        CompactionStyleMode::Leveled
    } else {
        CompactionStyleMode::Fifo
    }
}

fn storage_mode_mode(value: u8) -> StorageModeMode {
    if value == StorageModeMode::InMemory as u8 {
        StorageModeMode::InMemory
    } else if value == StorageModeMode::InMemoryPersistent as u8 {
        StorageModeMode::InMemoryPersistent
    } else {
        StorageModeMode::Disk
    }
}
