//! Range, watch, and administrative scope operations.

use bytes::Bytes;

use crate::error::Result;
use crate::pool::Backend;
use crate::watch::WatchStream;

use super::Scope;

impl Scope {
    /// Scan every key with `prefix`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the scan fails.
    pub async fn scan_prefix<K: AsRef<[u8]>>(&self, prefix: K) -> Result<Vec<(Bytes, Bytes)>> {
        let connection = self.pool.acquire().await?;
        connection.scan_prefix(self, prefix).await
    }

    /// Scan a half-open key range. `None` bounds are open.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the scan fails.
    pub async fn scan_range<K: AsRef<[u8]>>(
        &self,
        start: Option<K>,
        end: Option<K>,
    ) -> Result<Vec<(Bytes, Bytes)>> {
        let connection = self.pool.acquire().await?;
        connection.scan_range(self, start, end).await
    }

    /// Stream changes for every key with `prefix`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown.
    pub async fn watch_prefix<K: AsRef<[u8]>>(&self, prefix: K) -> Result<WatchStream> {
        self.watch_prefix_from(prefix, None).await
    }

    /// Resume a prefix watch after the sequence `from_seq`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the cursor expired.
    pub async fn watch_prefix_from<K: AsRef<[u8]>>(
        &self,
        prefix: K,
        from_seq: impl Into<Option<u64>>,
    ) -> Result<WatchStream> {
        let from_seq = from_seq.into();
        let resolved = self.resolve().await?;
        let prefix = Bytes::copy_from_slice(prefix.as_ref());
        let mut stream = match &self.pool.inner_backend() {
            Backend::Embedded(db) => {
                crate::watch::spawn_embedded(db.clone(), resolved.scope_id, prefix, from_seq)?
            }
            Backend::Networked(backend) => crate::watch::spawn_networked(
                backend.uri.clone(),
                resolved.scope_id,
                prefix,
                from_seq,
            ),
        };
        stream.wait_ready().await;
        Ok(stream)
    }

    /// Return a typed collection rooted at `prefix`.
    #[cfg(any(feature = "json", feature = "postcard"))]
    pub fn collection<T>(&self, prefix: impl AsRef<[u8]>) -> crate::collection::Collection<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        crate::collection::Collection::new(self.clone(), prefix.as_ref())
    }
}
