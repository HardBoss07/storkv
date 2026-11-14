use crate::format::{MAX_KEY_LEN, MAX_VAL_LEN};
use crate::mvcc::{TransactionContext, TxMutation};
use crate::types::{SequenceNumber, TimestampMs};
use crate::vfs::{FileSystem, PosixFs};
use crate::{MergeOp, StorKvError};
use bytes::Bytes;
use std::sync::Arc;
use std::time::Duration;

pub struct Transaction<FS: FileSystem + 'static = PosixFs> {
    pub(crate) db: Arc<crate::db::DbInner<FS>>,
    pub(crate) ctx: TransactionContext,
}

impl<FS: FileSystem + 'static> Transaction<FS> {
    pub fn read_seq(&self) -> u64 {
        self.ctx.read_seq
    }

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

    pub fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        key: K,
        val: V,
    ) -> crate::Result<()> {
        let user_key = key.as_ref();
        let entry_value = val.as_ref();
        validate_key(user_key)?;
        validate_value(entry_value)?;
        self.ctx.put(
            scope.scope_id,
            Bytes::copy_from_slice(user_key),
            Bytes::copy_from_slice(entry_value),
            0,
        );
        Ok(())
    }

    pub fn put_with_ttl<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        key: K,
        val: V,
        ttl: Duration,
    ) -> crate::Result<()> {
        let user_key = key.as_ref();
        let entry_value = val.as_ref();
        validate_key(user_key)?;
        validate_value(entry_value)?;

        let expires_at_ms = TimestampMs::now().get() + ttl.as_millis() as u64;
        self.ctx.put(
            scope.scope_id,
            Bytes::copy_from_slice(user_key),
            Bytes::copy_from_slice(entry_value),
            expires_at_ms,
        );
        Ok(())
    }

    pub fn delete<K: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        key: K,
    ) -> crate::Result<()> {
        let user_key = key.as_ref();
        validate_key(user_key)?;
        self.ctx
            .delete(scope.scope_id, Bytes::copy_from_slice(user_key));
        Ok(())
    }

    pub fn delete_range<K: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        start_key: K,
        end_key: K,
    ) -> crate::Result<()> {
        let range_start = start_key.as_ref();
        let range_end = end_key.as_ref();
        validate_key(range_start)?;
        validate_key(range_end)?;
        self.ctx.write_set.insert(
            (scope.scope_id, Bytes::copy_from_slice(range_start)),
            TxMutation::DeleteRange {
                end_key: Bytes::copy_from_slice(range_end),
            },
        );
        Ok(())
    }

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

    pub fn scan_prefix<K: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        prefix: K,
    ) -> crate::Result<Vec<(Bytes, Bytes)>> {
        self.db
            .scan_prefix(scope.scope_id, prefix.as_ref(), self.ctx.read_seq)
    }

    pub fn merge<K: AsRef<[u8]>>(
        &mut self,
        scope: &crate::ScopeHandle,
        key: K,
        op: MergeOp,
    ) -> crate::Result<()> {
        let operand = match op {
            MergeOp::CounterAdd(value) => Bytes::from(value.to_string()),
            MergeOp::Custom(value) => value,
        };
        self.ctx.merge(
            scope.scope_id,
            Bytes::copy_from_slice(key.as_ref()),
            operand,
        );
        Ok(())
    }

    pub fn commit(mut self) -> crate::Result<()> {
        let read_seq = self.ctx.read_seq;
        let context = std::mem::replace(&mut self.ctx, TransactionContext::new(0));
        let result = self.db.commit_tx(context);
        self.release_reader(read_seq);
        result
    }

    fn release_reader(&self, read_seq: u64) {
        let mut readers = self.db.active_tx_readers.write();
        if let Some(position) = readers.iter().position(|&seq| seq == read_seq) {
            readers.remove(position);
        }
    }
}

fn validate_key(user_key: &[u8]) -> crate::Result<()> {
    if user_key.len() > MAX_KEY_LEN {
        return Err(StorKvError::KeyTooLarge {
            max: MAX_KEY_LEN,
            actual: user_key.len(),
        });
    }
    Ok(())
}

fn validate_value(entry_value: &[u8]) -> crate::Result<()> {
    if entry_value.len() > MAX_VAL_LEN {
        return Err(StorKvError::ValueTooLarge {
            max: MAX_VAL_LEN,
            actual: entry_value.len(),
        });
    }
    Ok(())
}

impl<FS: FileSystem + 'static> Drop for Transaction<FS> {
    fn drop(&mut self) {
        let read_seq = self.ctx.read_seq;
        let mut readers = self.db.active_tx_readers.write();
        if let Some(position) = readers.iter().position(|&seq| seq == read_seq) {
            readers.remove(position);
        }
    }
}
