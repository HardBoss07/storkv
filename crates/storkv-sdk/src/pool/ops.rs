//! Transaction helpers and pooled point operations.

use crate::error::Result;
use crate::options::RetryPolicy;
use crate::scope::Scope;
use crate::transaction::Transaction;

use super::{PoolConnection, StorkvPool, TxFuture};

impl StorkvPool {
    /// Run a transaction closure with the default retry policy.
    ///
    /// # Errors
    ///
    /// Returns the closure error when it is not retryable, or when the retry
    /// budget runs out.
    pub async fn transaction<F, T>(&self, body: F) -> Result<T>
    where
        F: for<'a> Fn(&'a mut Transaction) -> TxFuture<'a, T> + Send + Sync,
        T: Send,
    {
        self.transaction_with(self.inner.options.retry.clone(), body)
            .await
    }

    /// Run a transaction closure with an explicit retry policy.
    ///
    /// # Errors
    ///
    /// Returns the closure error when it is not retryable, or when the retry
    /// budget runs out.
    pub async fn transaction_with<F, T>(&self, policy: RetryPolicy, body: F) -> Result<T>
    where
        F: for<'a> Fn(&'a mut Transaction) -> TxFuture<'a, T> + Send + Sync,
        T: Send,
    {
        let mut attempt = 0u32;
        loop {
            let connection = self.acquire().await?;
            let mut transaction = connection.begin().await?;
            match body(&mut transaction).await {
                Ok(value) => match transaction.commit().await {
                    Ok(()) => return Ok(value),
                    Err(error) if error.is_retryable() && attempt < policy.max_retries => {
                        tokio::time::sleep(policy.delay(attempt)).await;
                        attempt += 1;
                    }
                    Err(error) => return Err(error),
                },
                Err(error) => {
                    transaction.rollback().await;
                    if !error.is_retryable() || attempt >= policy.max_retries {
                        return Err(error);
                    }
                    tokio::time::sleep(policy.delay(attempt)).await;
                    attempt += 1;
                }
            }
        }
    }
}

impl PoolConnection {
    /// Set the consistency level on this connection's server.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub async fn set_consistency(&self, consistency: storkv_proto::Consistency) -> Result<()> {
        self.conn()?.set_consistency(consistency).await
    }

    /// Begin an explicit transaction on this connection.
    ///
    /// # Errors
    ///
    /// Returns an error when the backend cannot start a transaction.
    pub async fn begin(self) -> Result<Transaction> {
        Transaction::begin(self).await
    }

    /// Read one key from `scope`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the read fails.
    pub async fn get<K: AsRef<[u8]>>(&self, scope: &Scope, key: K) -> Result<Option<bytes::Bytes>> {
        let resolved = scope.resolve_via(self.conn()?).await?;
        self.conn()?.get(&resolved, key.as_ref()).await
    }

    /// Store one key/value pair in `scope`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the write fails.
    pub async fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &Scope,
        key: K,
        value: V,
    ) -> Result<()> {
        let resolved = scope.resolve_via(self.conn()?).await?;
        self.conn()?
            .put(
                &resolved,
                key.as_ref(),
                bytes::Bytes::copy_from_slice(value.as_ref()),
            )
            .await
    }

    /// Store one key/value pair with a time to live.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the write fails.
    pub async fn put_with_ttl<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &Scope,
        key: K,
        value: V,
        ttl: std::time::Duration,
    ) -> Result<()> {
        let resolved = scope.resolve_via(self.conn()?).await?;
        self.conn()?
            .put_with_ttl(
                &resolved,
                key.as_ref(),
                bytes::Bytes::copy_from_slice(value.as_ref()),
                ttl,
            )
            .await
    }

    /// Delete one key from `scope`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the write fails.
    pub async fn delete<K: AsRef<[u8]>>(&self, scope: &Scope, key: K) -> Result<()> {
        let resolved = scope.resolve_via(self.conn()?).await?;
        self.conn()?.delete(&resolved, key.as_ref()).await
    }

    /// Scan every key that starts with `prefix`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the scan fails.
    pub async fn scan_prefix<K: AsRef<[u8]>>(
        &self,
        scope: &Scope,
        prefix: K,
    ) -> Result<Vec<(bytes::Bytes, bytes::Bytes)>> {
        let resolved = scope.resolve_via(self.conn()?).await?;
        self.conn()?.scan_prefix(&resolved, prefix.as_ref()).await
    }

    /// Scan a half-open key range in `scope`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the scan fails.
    pub async fn scan_range<K: AsRef<[u8]>>(
        &self,
        scope: &Scope,
        start: Option<K>,
        end: Option<K>,
    ) -> Result<Vec<(bytes::Bytes, bytes::Bytes)>> {
        let resolved = scope.resolve_via(self.conn()?).await?;
        let start = start.map(|bound| bytes::Bytes::copy_from_slice(bound.as_ref()));
        let end = end.map(|bound| bytes::Bytes::copy_from_slice(bound.as_ref()));
        self.conn()?.scan_range(&resolved, start, end).await
    }
}
