//! Write-set buffering, commit, and rollback.

use std::time::Duration;

use bytes::Bytes;

use crate::error::Result;
use crate::scope::Scope;

use super::{Transaction, TxInner, resolved_client, resolved_core};

impl Transaction {
    /// Buffer a value write.
    ///
    /// # Errors
    ///
    /// Returns an error when the key or value is invalid.
    pub async fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &mut self,
        scope: &Scope,
        key: K,
        value: V,
    ) -> Result<()> {
        let resolved = self.resolve(scope).await?;
        match &mut self.inner {
            TxInner::Embedded(transaction) => transaction
                .put(&resolved_core(&resolved), key, value)
                .map_err(Into::into),
            TxInner::Networked(transaction) => {
                transaction.put(&resolved_client(&resolved), key, value);
                Ok(())
            }
        }
    }

    /// Buffer a value write with a time to live.
    ///
    /// # Errors
    ///
    /// Returns an error when the key or value is invalid.
    pub async fn put_with_ttl<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &mut self,
        scope: &Scope,
        key: K,
        value: V,
        ttl: Duration,
    ) -> Result<()> {
        let resolved = self.resolve(scope).await?;
        match &mut self.inner {
            TxInner::Embedded(transaction) => transaction
                .put_with_ttl(&resolved_core(&resolved), key, value, ttl)
                .map_err(Into::into),
            TxInner::Networked(transaction) => {
                transaction.put_with_ttl(&resolved_client(&resolved), key, value, ttl);
                Ok(())
            }
        }
    }

    /// Buffer a delete.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid.
    pub async fn delete<K: AsRef<[u8]>>(&mut self, scope: &Scope, key: K) -> Result<()> {
        let resolved = self.resolve(scope).await?;
        match &mut self.inner {
            TxInner::Embedded(transaction) => transaction
                .delete(&resolved_core(&resolved), key)
                .map_err(Into::into),
            TxInner::Networked(transaction) => {
                transaction.delete(&resolved_client(&resolved), key);
                Ok(())
            }
        }
    }

    /// Buffer a merge operation.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid or the operator is unknown.
    pub async fn merge<K: AsRef<[u8]>>(
        &mut self,
        scope: &Scope,
        key: K,
        op: storkv_core::MergeOp,
    ) -> Result<()> {
        let resolved = self.resolve(scope).await?;
        match &mut self.inner {
            TxInner::Embedded(transaction) => transaction
                .merge(&resolved_core(&resolved), key, op)
                .map_err(Into::into),
            TxInner::Networked(transaction) => {
                let op = crate::backend::wire_merge_op(&op)?;
                transaction.merge(&resolved_client(&resolved), key, op);
                Ok(())
            }
        }
    }

    /// Buffer an atomic integer counter increment.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid.
    pub async fn counter_add<K: AsRef<[u8]>>(
        &mut self,
        scope: &Scope,
        key: K,
        delta: i64,
    ) -> Result<()> {
        self.merge(scope, key, storkv_core::MergeOp::CounterAdd(delta))
            .await
    }

    /// Buffer a bounded or unbounded byte append.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid.
    pub async fn append_bytes<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &mut self,
        scope: &Scope,
        key: K,
        data: V,
        max_len: Option<usize>,
    ) -> Result<()> {
        self.merge(
            scope,
            key,
            storkv_core::MergeOp::AppendBytes {
                data: Bytes::copy_from_slice(data.as_ref()),
                max_len,
            },
        )
        .await
    }

    /// Buffer a monotonic `i64` maximum update.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid.
    pub async fn max_i64<K: AsRef<[u8]>>(
        &mut self,
        scope: &Scope,
        key: K,
        value: i64,
    ) -> Result<()> {
        self.merge(scope, key, storkv_core::MergeOp::MaxI64(value))
            .await
    }

    /// Buffer a monotonic `i64` minimum update.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid.
    pub async fn min_i64<K: AsRef<[u8]>>(
        &mut self,
        scope: &Scope,
        key: K,
        value: i64,
    ) -> Result<()> {
        self.merge(scope, key, storkv_core::MergeOp::MinI64(value))
            .await
    }

    /// Buffer a bitwise OR update on a `u64` value.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid.
    pub async fn bit_or<K: AsRef<[u8]>>(&mut self, scope: &Scope, key: K, bits: u64) -> Result<()> {
        self.merge(scope, key, storkv_core::MergeOp::BitOr(bits))
            .await
    }

    /// Buffer a bitwise AND update on a `u64` value.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid.
    pub async fn bit_and<K: AsRef<[u8]>>(
        &mut self,
        scope: &Scope,
        key: K,
        bits: u64,
    ) -> Result<()> {
        self.merge(scope, key, storkv_core::MergeOp::BitAnd(bits))
            .await
    }

    /// Buffer a half-open range delete.
    ///
    /// # Errors
    ///
    /// Returns an error when either bound is invalid.
    pub async fn delete_range<K: AsRef<[u8]>>(
        &mut self,
        scope: &Scope,
        start_key: K,
        end_key: K,
    ) -> Result<()> {
        let resolved = self.resolve(scope).await?;
        match &mut self.inner {
            TxInner::Embedded(transaction) => transaction
                .delete_range(&resolved_core(&resolved), start_key, end_key)
                .map_err(Into::into),
            TxInner::Networked(transaction) => {
                transaction.delete_range(&resolved_client(&resolved), start_key, end_key);
                Ok(())
            }
        }
    }

    /// Commit the transaction.
    ///
    /// # Errors
    ///
    /// Returns [`crate::SdkError::Core`] or [`crate::SdkError::Client`] with a
    /// write conflict when the read set changed after the reads.
    pub async fn commit(self) -> Result<()> {
        let Transaction { inner, connection } = self;
        let result = match inner {
            TxInner::Embedded(transaction) => transaction.commit().map_err(Into::into),
            TxInner::Networked(transaction) => transaction.commit().await.map_err(Into::into),
        };
        drop(connection);
        result
    }

    /// Roll back the transaction by discarding its write set.
    pub async fn rollback(self) {
        let Transaction { inner, connection } = self;
        drop(inner);
        drop(connection);
    }
}
