use std::collections::{BTreeMap, HashMap};

use super::codec::VersionEdit;
use super::record::{
    CompactionStyleMode, DurabilityMode, EditRecord, RetentionMode, ScopeMeta, SnapshotMeta,
    SstFileMeta,
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
    match value {
        0 => DurabilityMode::StrictFsync,
        _ => DurabilityMode::Buffered,
    }
}

fn retention_mode(value: u8) -> RetentionMode {
    match value {
        0 => RetentionMode::Permanent,
        _ => RetentionMode::Ttl,
    }
}

fn compaction_style_mode(value: u8) -> CompactionStyleMode {
    match value {
        0 => CompactionStyleMode::Leveled,
        _ => CompactionStyleMode::Fifo,
    }
}
