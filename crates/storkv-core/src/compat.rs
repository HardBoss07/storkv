//! Embedded drop-in adapters mirroring `sled`/`redb` ergonomics.
//!
//! A [`Tree`](crate::compat::Tree) is an ordinary StorKV scope: data written through it is visible to
//! every other transport, participates in MVCC, and is covered by backups. No
//! new storage format is introduced.

use bytes::Bytes;

use crate::db::{Db, ScopeHandle};
use crate::error::Result;
use crate::mvcc::MergeOp;
use crate::scope::ScopeOptions;
use crate::transaction::Transaction;

/// Engine-level operations shared by embedded adapters.
pub trait KvEngine {
    /// Open (creating if necessary) a named tree.
    fn open_tree(&self, name: &str) -> Result<Tree>;
    /// Flush pending writes. StorKV commits are already WAL-durable.
    fn flush(&self) -> Result<()>;
}

impl KvEngine for Db {
    fn open_tree(&self, name: &str) -> Result<Tree> {
        self.open_tree(name)
    }

    fn flush(&self) -> Result<()> {
        Ok(())
    }
}

impl Db {
    ///
    /// # Errors
    ///
    /// Returns `StorkvError::ScopeAlreadyExists` when a scope with `name` appears during creation.
    /// Returns `StorkvError::InvalidOperation` when `name` is not a valid scope name.
    /// Returns `StorkvError::Io` when the manifest cannot record the new scope.
    /// Open a named tree backed by a dedicated scope, creating it on first use.
    pub fn open_tree(&self, name: &str) -> Result<Tree> {
        if let Some(scope_id) = self.inner().scope_names.read().get(name).copied() {
            return Ok(Tree {
                db: self.clone(),
                scope: ScopeHandle {
                    scope_id,
                    name: name.to_string(),
                },
            });
        }
        let scope = self.create_scope(name, ScopeOptions::default())?;
        Ok(Tree {
            db: self.clone(),
            scope,
        })
    }
}

/// A named keyspace exposing a `sled`-style API.
#[derive(Clone)]
pub struct Tree {
    db: Db,
    scope: ScopeHandle,
}

impl Tree {
    /// Scope identifier backing this tree.
    pub fn scope_id(&self) -> u32 {
        self.scope.scope_id
    }

    /// Tree name.
    pub fn name(&self) -> &str {
        &self.scope.name
    }

    ///
    /// # Errors
    ///
    /// Returns `StorkvError::ScopeNotFound` or `StorkvError::ScopeIdNotFound` when the scope is missing.
    /// Returns `StorkvError::ScopeDropped` after the scope is dropped.
    /// Returns `StorkvError::Corruption` or `StorkvError::Io` when a read fails.
    /// Read one key. `None` means the key does not exist.
    pub fn get<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Bytes>> {
        self.db.get(&self.scope, key)
    }

    ///
    /// # Errors
    ///
    /// Returns the read errors of [`Tree::get`].
    /// Returns `StorkvError::KeyTooLarge` or `StorkvError::ValueTooLarge` when the entry exceeds a limit.
    /// Returns `StorkvError::WriteStalled` in non-blocking mode when a write watermark is exceeded.
    /// Returns `StorkvError::Io` when the commit cannot write the WAL.
    /// Insert a value, returning the previous value if any.
    pub fn insert<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        key: K,
        value: V,
    ) -> Result<Option<Bytes>> {
        let key = key.as_ref();
        let previous = self.db.get(&self.scope, key)?;
        self.db.put(&self.scope, key, value)?;
        Ok(previous)
    }

    ///
    /// # Errors
    ///
    /// Returns the read errors of [`Tree::get`].
    /// Returns `StorkvError::ScopeDropped` after the scope is dropped.
    /// Returns `StorkvError::Io` when the delete cannot write the WAL.
    /// Remove a key, returning the previous value if any.
    pub fn remove<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Bytes>> {
        let key = key.as_ref();
        let previous = self.db.get(&self.scope, key)?;
        self.db.delete(&self.scope, key)?;
        Ok(previous)
    }

    ///
    /// # Errors
    ///
    /// Returns `StorkvError::KeyTooLarge` or `StorkvError::ValueTooLarge` when the entry exceeds a limit.
    /// Returns `StorkvError::WriteStalled` in non-blocking mode when a write watermark is exceeded.
    /// Returns `StorkvError::ScopeNotFound` when the scope is missing.
    /// Returns `StorkvError::Io` when the conditional write fails.
    /// Compare-and-swap using an atomic conditional write.
    ///
    /// Returns `false` when the observed value does not match `expected`. An
    /// empty `new_value` deletes the key.
    pub fn compare_and_swap<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        key: K,
        expected: Option<&[u8]>,
        new_value: V,
    ) -> Result<bool> {
        let condition = match expected {
            Some(expected) => crate::PutCondition::ExpectedValue(Bytes::copy_from_slice(expected)),
            None => crate::PutCondition::Absent,
        };
        let value = match new_value.as_ref() {
            [] => None,
            value => Some(value),
        };
        match self.db.put_if(&self.scope, key, &condition, value, None)? {
            crate::PutIfResult::Applied { .. } => Ok(true),
            _ => Ok(false),
        }
    }

    ///
    /// # Errors
    ///
    /// Returns `StorkvError::ScopeNotFound` or `StorkvError::ScopeIdNotFound` when the scope is missing.
    /// Returns `StorkvError::Corruption` or `StorkvError::Io` when a table cannot be read.
    /// Read every key with the given prefix.
    pub fn scan_prefix<K: AsRef<[u8]>>(&self, prefix: K) -> Result<Vec<(Bytes, Bytes)>> {
        self.db.scan_prefix(&self.scope, prefix)
    }

    ///
    /// # Errors
    ///
    /// Returns `StorkvError::ScopeNotFound` or `StorkvError::ScopeIdNotFound` when the scope is missing.
    /// Returns `StorkvError::Corruption` or `StorkvError::Io` when a table cannot be read.
    /// Read a half-open key range.
    pub fn range<K: AsRef<[u8]>>(&self, start: K, end: K) -> Result<Vec<(Bytes, Bytes)>> {
        let start = start.as_ref().to_vec();
        let end = end.as_ref().to_vec();
        self.db.scan(&self.scope, Some(start), Some(end))
    }

    ///
    /// # Errors
    ///
    /// Returns `StorkvError::KeyTooLarge` when the key exceeds the limit.
    /// Returns `StorkvError::WriteStalled` in non-blocking mode when a write watermark is exceeded.
    /// Returns `StorkvError::InvalidOperation` when the merge operand is invalid.
    /// Returns `StorkvError::ScopeNotFound` when the scope is missing.
    /// Apply an atomic counter delta through the MVCC merge operator.
    pub fn merge<K: AsRef<[u8]>>(&self, key: K, delta: i64) -> Result<()> {
        self.db.merge(&self.scope, key, MergeOp::CounterAdd(delta))
    }

    /// Start a batch for this tree.
    pub fn batch(&self) -> Batch {
        Batch {
            tree: self.clone(),
            transaction: self.db.begin_tx(),
        }
    }

    ///
    /// # Errors
    ///
    /// Returns `StorkvError::WriteConflict` when an OCC read key changed.
    /// Returns `StorkvError::KeyTooLarge` or `StorkvError::ValueTooLarge` when an entry exceeds a limit.
    /// Returns `StorkvError::WriteStalled` in non-blocking mode when a write watermark is exceeded.
    /// Returns `StorkvError::Io` when the commit cannot write the WAL.
    /// Apply a buffered batch in a single atomic WAL frame.
    pub fn apply_batch(&self, batch: Batch) -> Result<()> {
        batch.transaction.commit()
    }

    ///
    /// # Errors
    ///
    /// Never returns an error. StorKV commits are durable when they return. The signature matches the adapter trait.
    /// Commit pending writes. StorKV writes are already durable.
    pub fn flush(&self) -> Result<()> {
        Ok(())
    }
}

/// Buffered writes applied atomically by [`Tree::apply_batch`].
pub struct Batch {
    tree: Tree,
    transaction: Transaction,
}

impl Batch {
    /// Buffer a value insert.
    pub fn insert<K: AsRef<[u8]>, V: AsRef<[u8]>>(&mut self, key: K, value: V) -> &mut Self {
        let _ = self
            .transaction
            .put(&self.tree.scope, key.as_ref(), value.as_ref());
        self
    }

    /// Buffer a key removal.
    pub fn remove<K: AsRef<[u8]>>(&mut self, key: K) -> &mut Self {
        let _ = self.transaction.delete(&self.tree.scope, key.as_ref());
        self
    }
}
