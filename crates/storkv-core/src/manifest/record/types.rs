use bytes::Bytes;

/// Magic bytes at the start of every manifest edit file.
pub const MANIFEST_MAGIC: &[u8; 8] = b"STORKMNF";
/// Current manifest record version.
pub const MANIFEST_VERSION: u16 = 1;

/// Durability mode stored in the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurabilityMode {
    /// Sync the WAL before every commit returns.
    StrictFsync = 0,
    /// Sync the WAL on a timer.
    Buffered = 1,
}

/// Retention mode stored in the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetentionMode {
    /// Keep keys until an explicit delete.
    Permanent = 0,
    /// Expire keys after a default TTL.
    Ttl = 1,
}

/// Compaction style stored in the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionStyleMode {
    /// Size-tiered levels L0 through L6.
    Leveled = 0,
    /// First-in, first-out retention by age.
    Fifo = 1,
}

/// Storage residency mode stored in the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageModeMode {
    /// Every structure lives on disk.
    Disk = 0,
    /// Every structure lives in RAM and the WAL is skipped.
    InMemory = 1,
    /// Memtables and SSTables live in RAM and mutations use the disk WAL.
    InMemoryPersistent = 2,
}

/// Persistent metadata of one scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeMeta {
    /// True after the scope is dropped.
    pub dropped: bool,
    /// Storage residency mode.
    pub storage_mode: StorageModeMode,
    /// Opaque scope identifier.
    pub scope_id: u32,
    /// Buffered durability sync interval in milliseconds.
    pub sync_ms: u32,
    // Fields ascend by binary width. Declaration order is not part of the
    // on-disk format.
    /// Default key lifetime in milliseconds. `0` means no expiry.
    pub default_ttl_ms: u64,
    /// Sequence number at which the scope was dropped. `0` means live.
    pub drop_seq: u64,
    /// Unique scope name.
    pub name: String,
    /// Durability mode.
    pub durability: DurabilityMode,
    /// Retention mode.
    pub retention: RetentionMode,
    /// Compaction style.
    pub compaction_style: CompactionStyleMode,
}

impl ScopeMeta {
    /// Typed view of the scope identifier.
    pub fn id(&self) -> crate::types::ScopeId {
        crate::types::ScopeId::new(self.scope_id)
    }
}

/// Persistent metadata of one SSTable file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstFileMeta {
    /// Compaction level from L0 through L6.
    pub level: u8,
    /// Identifier of the owning scope.
    pub scope_id: u32,
    // Fields ascend by binary width.
    /// SSTable file number.
    pub file_num: u64,
    /// File size in bytes.
    pub file_size: u64,
    /// Smallest sequence number in the file.
    pub min_seq: u64,
    /// Largest sequence number in the file.
    pub max_seq: u64,
    /// Largest absolute expiry timestamp in the file, or `0`.
    pub max_expires_at_ms: u64,
    /// Smallest user key in the file.
    pub min_key: Bytes,
    /// Largest user key in the file.
    pub max_key: Bytes,
}

impl SstFileMeta {
    /// Typed view of the SSTable file number.
    pub fn file_number(&self) -> crate::types::FileNumber {
        crate::types::FileNumber::new(self.file_num)
    }
}

/// Persistent metadata of one pinned snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMeta {
    /// Sequence number held by the snapshot.
    pub seq: u64,
    /// Unix epoch milliseconds when the snapshot was created.
    pub created_at_ms: u64,
    /// Unique snapshot name.
    pub name: String,
}

const _: () = assert!(std::mem::size_of::<ScopeMeta>() <= 64);
const _: () = assert!(std::mem::size_of::<SstFileMeta>() <= 128);
const _: () = assert!(std::mem::size_of::<SnapshotMeta>() <= 64);

/// One version edit in the manifest log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditRecord {
    /// Set the next sequence number to allocate.
    SetNextSequence(u64),
    /// Set the active WAL segment number.
    SetWalNumber(u64),
    /// Register a new scope.
    CreateScope {
        /// Opaque scope identifier.
        scope_id: u32,
        /// Durability mode code.
        durability: u8,
        /// Buffered durability sync interval in milliseconds.
        sync_ms: u32,
        /// Retention mode code.
        retention: u8,
        /// Default key lifetime in milliseconds.
        default_ttl_ms: u64,
        /// Compaction style code.
        compaction_style: u8,
        /// Storage residency mode code.
        storage_mode: u8,
        /// Unique scope name.
        name: String,
    },
    /// Mark a scope as dropped.
    DropScope {
        /// Opaque scope identifier.
        scope_id: u32,
        /// Sequence number of the drop.
        drop_seq: u64,
    },
    /// Add an SSTable file to the version.
    AddSstFile(SstFileMeta),
    /// Remove an SSTable file from the version.
    DeleteSstFile {
        /// Identifier of the owning scope.
        scope_id: u32,
        /// Level that holds the file.
        level: u8,
        /// File number to remove.
        file_num: u64,
    },
    /// Pin a snapshot.
    PinSnapshot {
        /// Sequence number to hold.
        seq: u64,
        /// Unix epoch milliseconds when the snapshot was created.
        created_at_ms: u64,
        /// Unique snapshot name.
        name: String,
    },
    /// Release a pinned snapshot.
    UnpinSnapshot {
        /// Name of the snapshot to release.
        name: String,
    },
    /// Register a range tombstone in the persistent index.
    AddRangeTombstone {
        /// Identifier of the owning scope.
        scope_id: u32,
        /// Sequence number of the delete.
        seq: u64,
        /// Inclusive start key.
        start_key: Bytes,
        /// Exclusive end key.
        end_key: Bytes,
    },
}
