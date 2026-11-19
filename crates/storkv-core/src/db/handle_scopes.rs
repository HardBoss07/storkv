//! Scope, snapshot, and maintenance operations of a database handle.

use super::{Db, ScopeHandle};

impl Db {
    /// Create a scope and return its handle.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorkvError::ScopeAlreadyExists`] when the name is taken.
    pub fn create_scope<S: Into<String>>(
        &self,
        name: S,
        options: crate::ScopeOptions,
    ) -> crate::Result<ScopeHandle> {
        self.inner.create_scope(name.into(), options)
    }

    /// Drop a scope and delete its data.
    ///
    /// # Errors
    ///
    /// Returns [`crate::StorkvError::ScopeNotFound`] when the name is unknown.
    pub fn drop_scope<S: AsRef<str>>(&self, name: S) -> crate::Result<()> {
        self.inner.drop_scope(name.as_ref())
    }

    /// Pin a snapshot at the current sequence number.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest write fails.
    pub fn create_snapshot<S: Into<String>>(&self, name: S) -> crate::Result<()> {
        self.inner.create_snapshot(name.into())
    }

    /// Release a pinned snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest write fails.
    pub fn unpin_snapshot<S: AsRef<str>>(&self, name: S) -> crate::Result<()> {
        self.inner.unpin_snapshot(name.as_ref())
    }

    /// Flush every scope and write recovery snapshots.
    ///
    /// # Errors
    ///
    /// Returns an error when a flush or a snapshot write fails.
    pub fn flush(&self) -> crate::Result<()> {
        self.inner.flush_all()
    }

    /// Run compaction on every scope immediately.
    ///
    /// # Errors
    ///
    /// Returns an error when compaction or manifest logging fails.
    pub fn compact(&self) -> crate::Result<()> {
        self.inner.run_compaction()
    }
}
