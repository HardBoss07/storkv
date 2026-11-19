//! Typed CRUD, scan, and watch operations for a collection.

use std::marker::PhantomData;
use std::time::Duration;

use bytes::Bytes;

use crate::error::{Result, SdkError};

use super::Collection;
use super::codec::TypedWatchStream;

impl<T> Collection<T>
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    /// Return the scope that contains this collection.
    #[must_use]
    pub fn scope(&self) -> &crate::scope::Scope {
        &self.scope
    }

    /// Return the collection prefix.
    #[must_use]
    pub fn prefix(&self) -> &[u8] {
        &self.prefix
    }

    pub(super) fn key(&self, key: &[u8]) -> Bytes {
        let mut combined = Vec::with_capacity(self.prefix.len() + key.len());
        combined.extend_from_slice(&self.prefix);
        combined.extend_from_slice(key);
        Bytes::from(combined)
    }

    fn decode_entry(&self, key: &[u8], value: &[u8]) -> Result<(Bytes, T)> {
        let stripped = key
            .strip_prefix(self.prefix.as_ref())
            .ok_or_else(|| SdkError::Serialization("entry outside collection prefix".into()))?;
        Ok((
            Bytes::copy_from_slice(stripped),
            self.codec.decode::<T>(value)?,
        ))
    }

    /// Read and decode one value.
    ///
    /// # Errors
    ///
    /// Returns an error when the read fails or the bytes are not valid for the
    /// selected codec.
    pub async fn get<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<T>> {
        let value = self.scope.get(self.key(key.as_ref())).await?;
        match value {
            Some(bytes) => Ok(Some(self.codec.decode::<T>(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Return `true` when the key exists.
    ///
    /// # Errors
    ///
    /// Returns an error when the read fails.
    pub async fn exists<K: AsRef<[u8]>>(&self, key: K) -> Result<bool> {
        self.scope.exists(self.key(key.as_ref())).await
    }

    /// Encode and store one value.
    ///
    /// # Errors
    ///
    /// Returns an error when encoding or the write fails.
    pub async fn put<K: AsRef<[u8]>>(&self, key: K, value: &T) -> Result<()> {
        let encoded = self.codec.encode(value)?;
        self.scope.put(self.key(key.as_ref()), encoded).await
    }

    /// Encode and store one value with a time to live.
    ///
    /// # Errors
    ///
    /// Returns an error when encoding or the write fails.
    pub async fn put_with_ttl<K: AsRef<[u8]>>(
        &self,
        key: K,
        value: &T,
        ttl: Duration,
    ) -> Result<()> {
        let encoded = self.codec.encode(value)?;
        self.scope
            .put_with_ttl(self.key(key.as_ref()), encoded, ttl)
            .await
    }

    /// Read one value and restart its time to live.
    ///
    /// # Errors
    ///
    /// Returns an error when the read, decode, or write fails.
    pub async fn get_and_touch<K: AsRef<[u8]>>(&self, key: K, ttl: Duration) -> Result<Option<T>> {
        let value = self
            .scope
            .get_and_touch(self.key(key.as_ref()), ttl)
            .await?;
        match value {
            Some(bytes) => Ok(Some(self.codec.decode::<T>(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Delete one value.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails.
    pub async fn delete<K: AsRef<[u8]>>(&self, key: K) -> Result<()> {
        self.scope.delete(self.key(key.as_ref())).await
    }

    /// Decode every value whose key starts with `prefix`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scan or any decode fails.
    pub async fn scan_prefix<K: AsRef<[u8]>>(&self, prefix: K) -> Result<Vec<(Bytes, T)>> {
        let entries = self.scope.scan_prefix(self.key(prefix.as_ref())).await?;
        entries
            .iter()
            .map(|(key, value)| self.decode_entry(key, value))
            .collect()
    }

    /// Decode every value in the collection.
    ///
    /// # Errors
    ///
    /// Returns an error when the scan or any decode fails.
    pub async fn scan(&self) -> Result<Vec<(Bytes, T)>> {
        self.scan_prefix([]).await
    }

    /// Decode every value in a half-open key range.
    ///
    /// # Errors
    ///
    /// Returns an error when the scan or any decode fails.
    pub async fn scan_range<K: AsRef<[u8]>>(
        &self,
        start: Option<K>,
        end: Option<K>,
    ) -> Result<Vec<(Bytes, T)>> {
        let start = start.map(|bound| self.key(bound.as_ref()));
        let end = end.map(|bound| self.key(bound.as_ref()));
        let entries = self.scope.scan_range(start, end).await?;
        entries
            .iter()
            .map(|(key, value)| self.decode_entry(key, value))
            .collect()
    }

    /// Stream decoded changes for keys under `prefix`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope does not resolve.
    pub async fn watch_prefix<K: AsRef<[u8]>>(&self, prefix: K) -> Result<TypedWatchStream<T>> {
        self.watch_prefix_from(prefix, None).await
    }

    /// Resume decoded prefix changes after the sequence `from_seq`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope does not resolve or the cursor expired.
    pub async fn watch_prefix_from<K: AsRef<[u8]>>(
        &self,
        prefix: K,
        from_seq: impl Into<Option<u64>>,
    ) -> Result<TypedWatchStream<T>> {
        let stream = self
            .scope
            .watch_prefix_from(self.key(prefix.as_ref()), from_seq)
            .await?;
        Ok(TypedWatchStream {
            inner: stream,
            strip: self.prefix.clone(),
            codec: self.codec,
            marker: PhantomData,
        })
    }
}
