//! Read and write views shared by direct and transactional Redis commands.

use bytes::Bytes;
use storkv_core::{Db, ScopeHandle, StorkvError, Transaction};

use super::super::keyspace::Atomic;

/// Read operations needed by hash and set commands.
pub(crate) trait ReadView {
    fn get(&mut self, scope: &ScopeHandle, key: &[u8]) -> Result<Option<Bytes>, StorkvError>;
    fn scan_prefix(
        &mut self,
        scope: &ScopeHandle,
        prefix: &[u8],
    ) -> Result<Vec<(Bytes, Bytes)>, StorkvError>;
}

/// Write operations needed by hash and set commands.
pub(crate) trait WriteView: ReadView {
    fn put(
        &mut self,
        scope: &ScopeHandle,
        key: &[u8],
        value: &[u8],
        expires_at_ms: u64,
    ) -> Result<(), StorkvError>;
    fn delete(&mut self, scope: &ScopeHandle, key: &[u8]) -> Result<(), StorkvError>;
}

/// Read-only view over a committed database snapshot.
pub(crate) struct DbView<'a>(pub(crate) &'a Db);

impl ReadView for DbView<'_> {
    fn get(&mut self, scope: &ScopeHandle, key: &[u8]) -> Result<Option<Bytes>, StorkvError> {
        self.0.get(scope, key)
    }

    fn scan_prefix(
        &mut self,
        scope: &ScopeHandle,
        prefix: &[u8],
    ) -> Result<Vec<(Bytes, Bytes)>, StorkvError> {
        self.0.scan_prefix(scope, prefix)
    }
}

impl ReadView for Atomic<'_> {
    fn get(&mut self, scope: &ScopeHandle, key: &[u8]) -> Result<Option<Bytes>, StorkvError> {
        Atomic::get(self, scope, key)
    }

    fn scan_prefix(
        &mut self,
        scope: &ScopeHandle,
        prefix: &[u8],
    ) -> Result<Vec<(Bytes, Bytes)>, StorkvError> {
        Atomic::scan_prefix(self, scope, prefix)
    }
}

impl WriteView for Atomic<'_> {
    fn put(
        &mut self,
        scope: &ScopeHandle,
        key: &[u8],
        value: &[u8],
        expires_at_ms: u64,
    ) -> Result<(), StorkvError> {
        Atomic::put(self, scope, key, value, expires_at_ms);
        Ok(())
    }

    fn delete(&mut self, scope: &ScopeHandle, key: &[u8]) -> Result<(), StorkvError> {
        Atomic::delete(self, scope, key);
        Ok(())
    }
}

impl ReadView for Transaction {
    fn get(&mut self, scope: &ScopeHandle, key: &[u8]) -> Result<Option<Bytes>, StorkvError> {
        Transaction::get(self, scope, key)
    }

    fn scan_prefix(
        &mut self,
        scope: &ScopeHandle,
        prefix: &[u8],
    ) -> Result<Vec<(Bytes, Bytes)>, StorkvError> {
        Transaction::scan_prefix(self, scope, prefix)
    }
}

impl WriteView for Transaction {
    fn put(
        &mut self,
        scope: &ScopeHandle,
        key: &[u8],
        value: &[u8],
        expires_at_ms: u64,
    ) -> Result<(), StorkvError> {
        self.put_with_expires_at(scope, key, value, expires_at_ms)
    }

    fn delete(&mut self, scope: &ScopeHandle, key: &[u8]) -> Result<(), StorkvError> {
        Transaction::delete(self, scope, key)
    }
}
