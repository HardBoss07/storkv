//! Snapshot reads for an optimistic transaction.

use bytes::Bytes;

use crate::mvcc::TxMutation;
use crate::types::SequenceNumber;

use super::Transaction;
use crate::vfs::FileSystem;

impl<FS: FileSystem + 'static> Transaction<FS> {
    /// Return the sequence number that the transaction reads at.
    pub fn read_seq(&self) -> u64 {
        self.ctx.read_seq
    }

    /// Read one key and add it to the OCC read set.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage read fails.
    pub fn get<K: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        key: K,
    ) -> crate::Result<Option<Bytes>> {
        self.ctx
            .record_read(scope.scope_id, Bytes::copy_from_slice(key.as_ref()));

        if let Some(mutation) = self.ctx.get_local(scope.scope_id, key.as_ref()) {
            return self.local_value(scope, key.as_ref(), mutation);
        }

        self.db.get_at(
            scope.scope_id,
            key.as_ref(),
            SequenceNumber::new(self.ctx.read_seq),
        )
    }

    fn local_value(
        &self,
        scope: &crate::ScopeHandle,
        key: &[u8],
        mutation: &TxMutation,
    ) -> crate::Result<Option<Bytes>> {
        match mutation {
            TxMutation::Put { val, .. } => Ok(Some(val.clone())),
            TxMutation::Delete => Ok(None),
            TxMutation::Merge { operand } => {
                let base =
                    self.db
                        .get_at(scope.scope_id, key, SequenceNumber::new(self.ctx.read_seq))?;
                let merged = self.db.merge_operator.full_merge(
                    key,
                    base.as_deref(),
                    std::slice::from_ref(operand),
                )?;
                Ok(Some(merged))
            }
            TxMutation::DeleteRange { .. } => Ok(None),
        }
    }

    /// Read an ordered key range at the transaction snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage read fails.
    pub fn scan<K: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        start_key: Option<K>,
        end_key: Option<K>,
    ) -> crate::Result<Vec<(Bytes, Bytes)>> {
        self.db.scan(
            scope.scope_id,
            start_key.as_ref().map(|key| key.as_ref()),
            end_key.as_ref().map(|key| key.as_ref()),
            self.ctx.read_seq,
        )
    }

    /// Read every key with the given prefix at the transaction snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage read fails.
    pub fn scan_prefix<K: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        prefix: K,
    ) -> crate::Result<Vec<(Bytes, Bytes)>> {
        self.db
            .scan_prefix(scope.scope_id, prefix.as_ref(), self.ctx.read_seq)
    }
}
