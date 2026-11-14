use bytes::Bytes;

pub const MANIFEST_MAGIC: &[u8; 8] = b"STORKMNF";
pub const MANIFEST_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurabilityMode {
    StrictFsync = 0,
    Buffered = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetentionMode {
    Permanent = 0,
    Ttl = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionStyleMode {
    Leveled = 0,
    Fifo = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeMeta {
    // Widest fields first: two u64, one owning String, two u32, three 1-byte
    // enums and a bool. Declaration order is not part of the on-disk format.
    pub default_ttl_ms: u64,
    pub drop_seq: u64,
    pub name: String,
    pub scope_id: u32,
    pub sync_ms: u32,
    pub durability: DurabilityMode,
    pub retention: RetentionMode,
    pub compaction_style: CompactionStyleMode,
    pub dropped: bool,
}

impl ScopeMeta {
    /// Typed view of the scope identifier.
    pub fn id(&self) -> crate::types::ScopeId {
        crate::types::ScopeId::new(self.scope_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstFileMeta {
    // Five u64 counters, then two owning byte strings, then the identifiers.
    pub file_num: u64,
    pub file_size: u64,
    pub min_seq: u64,
    pub max_seq: u64,
    pub max_expires_at_ms: u64,
    pub min_key: Bytes,
    pub max_key: Bytes,
    pub scope_id: u32,
    pub level: u8,
}

impl SstFileMeta {
    /// Typed view of the SSTable file number.
    pub fn file_number(&self) -> crate::types::FileNumber {
        crate::types::FileNumber::new(self.file_num)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMeta {
    pub seq: u64,
    pub created_at_ms: u64,
    pub name: String,
}

const _: () = assert!(std::mem::size_of::<ScopeMeta>() <= 64);
const _: () = assert!(std::mem::size_of::<SstFileMeta>() <= 128);
const _: () = assert!(std::mem::size_of::<SnapshotMeta>() <= 64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditRecord {
    SetNextSequence(u64),
    SetWalNumber(u64),
    CreateScope {
        scope_id: u32,
        durability: u8,
        sync_ms: u32,
        retention: u8,
        default_ttl_ms: u64,
        compaction_style: u8,
        name: String,
    },
    DropScope {
        scope_id: u32,
        drop_seq: u64,
    },
    AddSstFile(SstFileMeta),
    DeleteSstFile {
        scope_id: u32,
        level: u8,
        file_num: u64,
    },
    PinSnapshot {
        seq: u64,
        created_at_ms: u64,
        name: String,
    },
    UnpinSnapshot {
        name: String,
    },
}
