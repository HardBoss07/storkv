//! Cached read and write paths for a scope.

mod merge;

use std::time::Duration;

use bytes::Bytes;

use crate::error::Result;

use super::Scope;

impl Scope {
    /// Read one key. `None` means the key does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the read fails.
    pub async fn get<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Bytes>> {
        let key = key.as_ref();
        if self.cache.enabled() {
            self.activate_cache().await?;
            if let Some(value) = self.cache.lookup(key) {
                return Ok(value);
            }
        }
        let connection = self.pool.acquire().await?;
        let value = connection.get(self, key).await?;
        if self.cache.enabled() {
            if value.is_some() {
                if let Some(meta) = connection.key_meta(self, key).await? {
                    self.cache
                        .store_with_expires_at_ms(key, value.clone(), meta.expires_at_ms);
                    return Ok(value);
                }
                self.cache.store(key, None);
                return Ok(None);
            }
            self.cache.store(key, None);
        }
        Ok(value)
    }

    async fn activate_cache(&self) -> Result<()> {
        if self.cache.enabled() {
            let resolved = self.resolve().await?;
            self.cache.ensure_watcher(&self.pool, resolved.scope_id);
        }
        Ok(())
    }

    /// Return `true` when the key exists.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the read fails.
    pub async fn exists<K: AsRef<[u8]>>(&self, key: K) -> Result<bool> {
        Ok(self.get(key).await?.is_some())
    }

    /// Store one key/value pair.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the write fails.
    pub async fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(&self, key: K, value: V) -> Result<()> {
        self.activate_cache().await?;
        let key = key.as_ref();
        let cached = if self.cache.enabled() {
            Some(Bytes::copy_from_slice(value.as_ref()))
        } else {
            None
        };
        let connection = self.pool.acquire().await?;
        connection.put(self, key, value).await?;
        if let Some(cached) = cached {
            self.cache.store(key, Some(cached));
        }
        Ok(())
    }

    /// Store one key/value pair with a time to live.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the write fails.
    pub async fn put_with_ttl<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        key: K,
        value: V,
        ttl: Duration,
    ) -> Result<()> {
        self.activate_cache().await?;
        let key = key.as_ref();
        let cached = if self.cache.enabled() {
            Some(Bytes::copy_from_slice(value.as_ref()))
        } else {
            None
        };
        let connection = self.pool.acquire().await?;
        connection.put_with_ttl(self, key, value, ttl).await?;
        if let Some(cached) = cached {
            self.cache.store_with_ttl(key, Some(cached), ttl);
        }
        Ok(())
    }

    /// Conditionally store one key/value pair in this scope.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the write fails.
    pub async fn put_if<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        key: K,
        condition: &storkv_core::PutCondition,
        value: V,
        ttl: Option<Duration>,
    ) -> Result<storkv_core::PutIfResult> {
        self.activate_cache().await?;
        let connection = self.pool.acquire().await?;
        let result = connection
            .put_if(self, key.as_ref(), condition, value, ttl)
            .await?;
        self.cache.invalidate(key.as_ref());
        Ok(result)
    }

    /// Read one key and restart its time to live.
    ///
    /// The read and the rewrite run inside one OCC transaction so the returned
    /// value and the refreshed expiry always agree.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the operation fails.
    pub async fn get_and_touch<K: AsRef<[u8]>>(
        &self,
        key: K,
        ttl: Duration,
    ) -> Result<Option<Bytes>> {
        let scope = self.clone();
        let key = key.as_ref().to_vec();
        let cache_key = key.clone();
        let value = self
            .pool
            .transaction(move |transaction| {
                let scope = scope.clone();
                let key = key.clone();
                Box::pin(async move {
                    let value = transaction.get(&scope, &key).await?;
                    if let Some(current) = value.as_ref() {
                        transaction.put_with_ttl(&scope, &key, current, ttl).await?;
                    }
                    Ok(value)
                })
            })
            .await?;
        self.cache.invalidate(&cache_key);
        Ok(value)
    }

    /// Delete one key.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the write fails.
    pub async fn delete<K: AsRef<[u8]>>(&self, key: K) -> Result<()> {
        self.activate_cache().await?;
        let key = key.as_ref();
        let connection = self.pool.acquire().await?;
        connection.delete(self, key).await?;
        if self.cache.enabled() {
            self.cache.store(key, None);
        }
        Ok(())
    }
}
