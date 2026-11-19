//! Write-set buffering, commit, and rollback.

use std::time::Duration;

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
