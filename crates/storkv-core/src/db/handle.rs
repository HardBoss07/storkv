use crate::db::DbInner;
use crate::options::DbOptions;
use crate::telemetry::Telemetry;
use crate::vfs::PosixFs;
use crate::watch::ChangeEvent;
use bytes::Bytes;
use fs2::FileExt;
use std::fs::File as StdFile;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ScopeHandle {
    pub scope_id: u32,
    pub name: String,
}

impl ScopeHandle {
    /// Typed view of the scope identifier.
    pub fn id(&self) -> crate::types::ScopeId {
        crate::types::ScopeId::new(self.scope_id)
    }
}

pub struct Db {
    inner: Arc<DbInner<PosixFs>>,
    _lock_file: StdFile,
}

impl Db {
    pub fn open<P: AsRef<Path>>(path: P, options: DbOptions) -> crate::Result<Self> {
        let db_root = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&db_root)?;

        let lock_path = db_root.join("LOCK");
        let lock_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&lock_path)?;

        lock_file
            .try_lock_exclusive()
            .map_err(|_| crate::StorKvError::DatabaseLocked(lock_path.clone()))?;

        let fs = Arc::new(PosixFs);
        let inner = DbInner::open(fs, db_root, options)?;

        Ok(Self {
            inner: Arc::new(inner),
            _lock_file: lock_file,
        })
    }

    pub fn create_scope<S: Into<String>>(
        &self,
        name: S,
        options: crate::ScopeOptions,
    ) -> crate::Result<ScopeHandle> {
        self.inner.create_scope(name.into(), options)
    }

    pub fn drop_scope<S: AsRef<str>>(&self, name: S) -> crate::Result<()> {
        self.inner.drop_scope(name.as_ref())
    }

    pub fn begin_tx(&self) -> crate::Transaction {
        self.inner.begin_tx()
    }

    pub fn get<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> crate::Result<Option<Bytes>> {
        self.inner.get(scope.scope_id, key.as_ref())
    }

    pub fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
    ) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.put(scope, key, val)?;
        tx.commit()
    }

    pub fn put_with_ttl<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        val: V,
        ttl: Duration,
    ) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.put_with_ttl(scope, key, val, ttl)?;
        tx.commit()
    }

    pub fn delete<K: AsRef<[u8]>>(&self, scope: &ScopeHandle, key: K) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.delete(scope, key)?;
        tx.commit()
    }

    pub fn merge<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        op: crate::MergeOp,
    ) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.merge(scope, key, op)?;
        tx.commit()
    }

    pub fn create_snapshot<S: Into<String>>(&self, name: S) -> crate::Result<()> {
        self.inner.create_snapshot(name.into())
    }

    pub fn checkpoint<P: AsRef<Path>>(&self, dest_dir: P) -> crate::Result<()> {
        self.inner.checkpoint(dest_dir.as_ref())
    }

    pub fn unpin_snapshot<S: AsRef<str>>(&self, name: S) -> crate::Result<()> {
        self.inner.unpin_snapshot(name.as_ref())
    }

    pub fn delete_range<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        start_key: K,
        end_key: K,
    ) -> crate::Result<()> {
        let mut tx = self.begin_tx();
        tx.delete_range(scope, start_key, end_key)?;
        tx.commit()
    }

    pub fn scan<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        start_key: Option<K>,
        end_key: Option<K>,
    ) -> crate::Result<Vec<(Bytes, Bytes)>> {
        self.inner.scan(
            scope.scope_id,
            start_key.as_ref().map(|key| key.as_ref()),
            end_key.as_ref().map(|key| key.as_ref()),
            self.inner.last_visible_seq.load(Ordering::Acquire),
        )
    }

    pub fn scan_prefix<K: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        prefix: K,
    ) -> crate::Result<Vec<(Bytes, Bytes)>> {
        self.inner.scan_prefix(
            scope.scope_id,
            prefix.as_ref(),
            self.inner.last_visible_seq.load(Ordering::Acquire),
        )
    }

    pub fn telemetry(&self) -> &Telemetry {
        &self.inner.telemetry
    }

    pub fn inner(&self) -> &Arc<DbInner<PosixFs>> {
        &self.inner
    }

    pub fn watch(&self) -> crossbeam_channel::Receiver<ChangeEvent> {
        self.inner.watch_registry.subscribe()
    }

    pub fn compact(&self) -> crate::Result<()> {
        self.inner.run_compaction()
    }
}
