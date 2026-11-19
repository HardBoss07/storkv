//! Read-view lookup inside a transaction.

use bytes::Bytes;

use crate::error::Result;
use crate::scope::Scope;

use super::{Transaction, TxInner, resolved_client, resolved_core};

impl Transaction {
    /// Read one key inside the transaction and add it to the read set.
    ///
    /// # Errors
    ///
    /// Returns an error when the read fails.
    pub async fn get<K: AsRef<[u8]>>(&mut self, scope: &Scope, key: K) -> Result<Option<Bytes>> {
        let resolved = self.resolve(scope).await?;
        match &mut self.inner {
            TxInner::Embedded(transaction) => transaction
                .get(&resolved_core(&resolved), key)
                .map_err(Into::into),
            TxInner::Networked(transaction) => transaction
                .get(&resolved_client(&resolved), key)
                .await
                .map_err(Into::into),
        }
    }

    /// Return `true` when the key exists inside the transaction snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the read fails.
    pub async fn exists<K: AsRef<[u8]>>(&mut self, scope: &Scope, key: K) -> Result<bool> {
        Ok(self.get(scope, key).await?.is_some())
    }

    /// Scan every key with `prefix` at the transaction snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the scan fails.
    pub async fn scan_prefix<K: AsRef<[u8]>>(
        &mut self,
        scope: &Scope,
        prefix: K,
    ) -> Result<Vec<(Bytes, Bytes)>> {
        let resolved = self.resolve(scope).await?;
        match &mut self.inner {
            TxInner::Embedded(transaction) => transaction
                .scan_prefix(&resolved_core(&resolved), prefix)
                .map_err(Into::into),
            TxInner::Networked(transaction) => transaction
                .scan_prefix(&resolved_client(&resolved), prefix)
                .await
                .map_err(Into::into),
        }
    }

    /// Scan a half-open key range at the transaction snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the scan fails.
    pub async fn scan_range<K: AsRef<[u8]>>(
        &mut self,
        scope: &Scope,
        start: Option<K>,
        end: Option<K>,
    ) -> Result<Vec<(Bytes, Bytes)>> {
        let resolved = self.resolve(scope).await?;
        match &mut self.inner {
            TxInner::Embedded(transaction) => transaction
                .scan(&resolved_core(&resolved), start, end)
                .map_err(Into::into),
            TxInner::Networked(transaction) => transaction
                .scan(&resolved_client(&resolved), start, end)
                .await
                .map_err(Into::into),
        }
    }
}
