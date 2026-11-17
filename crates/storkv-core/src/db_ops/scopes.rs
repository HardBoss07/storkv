use crate::db::{DbInner, ScopeHandle};
use crate::manifest::{EditRecord, VersionEdit};
use crate::mvcc::TransactionContext;
use crate::scope::Scope;
use crate::vfs::FileSystem;
use crate::wal::{Durability, WalBatchFrame, WalOp};
use crate::watch::ChangeEvent;
use crate::{CompactionStyle, Retention, StorKvError};
use std::sync::Arc;
use std::sync::atomic::Ordering;

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Create a scope and persist it in the manifest.
    ///
    /// # Errors
    ///
    /// Returns [`StorKvError::ScopeAlreadyExists`] when the name is taken.
    pub fn create_scope(
        &self,
        name: String,
        options: crate::ScopeOptions,
    ) -> crate::Result<ScopeHandle> {
        let mut names = self.scope_names.write();
        if names.contains_key(&name) {
            return Err(StorKvError::ScopeAlreadyExists(name));
        }

        let mut scopes = self.scopes.write();
        let scope_id = (scopes.len() as u32) + 1;

        let scope = Arc::new(Scope::new(
            scope_id,
            name.clone(),
            options.clone(),
            self.fs.clone(),
            &self.scopes_dir,
        )?);
        scopes.insert(scope_id, scope);
        names.insert(name.clone(), scope_id);

        let manifest_edit = scope_creation_edit(scope_id, &name, &options);
        self.manifest_mgr.log_edit(manifest_edit)?;

        Ok(ScopeHandle { scope_id, name })
    }

    /// Drop a scope, record the drop in the WAL and manifest, and notify
    /// watchers.
    ///
    /// # Errors
    ///
    /// Returns [`StorKvError::ScopeNotFound`] when the name is unknown.
    pub fn drop_scope(&self, name: &str) -> crate::Result<()> {
        let scope_id = remove_scope_name(self, name)?;
        let drop_seq = self.last_visible_seq.fetch_add(1, Ordering::SeqCst) + 1;

        if let Some(scope) = self.scopes.read().get(&scope_id) {
            scope.mark_dropped(drop_seq);
        }

        write_drop_scope_to_wal(self, scope_id, drop_seq)?;
        log_drop_scope_edit(self, scope_id, drop_seq)?;
        self.watch_registry.broadcast(ChangeEvent::DropScope {
            scope_id,
            seq: drop_seq,
        });

        Ok(())
    }

    /// Begin a transaction at the newest committed sequence number.
    pub fn begin_tx(self: &Arc<Self>) -> crate::Transaction<FS> {
        let read_seq = {
            let _guard = self.commit_lock.lock();
            self.last_visible_seq.load(Ordering::Acquire)
        };
        self.begin_tx_at(read_seq)
    }

    /// Begin a transaction pinned to `read_seq`, retaining the snapshot for
    /// compaction while the transaction is alive.
    pub fn begin_tx_at(self: &Arc<Self>, read_seq: u64) -> crate::Transaction<FS> {
        self.active_tx_readers.write().push(read_seq);
        crate::Transaction {
            db: self.clone(),
            ctx: TransactionContext::new(read_seq),
        }
    }
}

fn remove_scope_name<FS: FileSystem>(db: &DbInner<FS>, name: &str) -> crate::Result<u32> {
    let mut names = db.scope_names.write();
    names
        .remove(name)
        .ok_or_else(|| StorKvError::ScopeNotFound(name.to_string()))
}

fn write_drop_scope_to_wal<FS: FileSystem>(
    db: &DbInner<FS>,
    scope_id: u32,
    drop_seq: u64,
) -> crate::Result<()> {
    let frame = WalBatchFrame::new(drop_seq, vec![WalOp::DropScope { scope_id }]);
    db.wal_mgr.write_batch(&frame, Durability::StrictFsync)
}

fn log_drop_scope_edit<FS: FileSystem>(
    db: &DbInner<FS>,
    scope_id: u32,
    drop_seq: u64,
) -> crate::Result<()> {
    let mut edit = VersionEdit::new();
    edit.add(EditRecord::DropScope { scope_id, drop_seq });
    db.manifest_mgr.log_edit(edit)
}

fn scope_creation_edit(scope_id: u32, name: &str, options: &crate::ScopeOptions) -> VersionEdit {
    let durability = match options.durability {
        Durability::StrictFsync => 0,
        Durability::Buffered { .. } => 1,
    };
    let sync_ms = match options.durability {
        Durability::Buffered { sync_interval } => sync_interval.as_millis() as u32,
        _ => 0,
    };
    let (retention, default_ttl_ms, compaction_style) = match options.retention {
        Retention::Permanent => (0, 0, 0),
        Retention::Ttl {
            default_ttl,
            compaction_style,
        } => (
            1,
            default_ttl.as_millis() as u64,
            match compaction_style {
                CompactionStyle::Leveled => 0,
                CompactionStyle::Fifo => 1,
            },
        ),
    };

    let mut edit = VersionEdit::new();
    edit.add(EditRecord::CreateScope {
        scope_id,
        durability,
        sync_ms,
        retention,
        default_ttl_ms,
        compaction_style,
        name: name.to_string(),
    });
    edit
}
