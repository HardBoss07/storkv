//! Merge and counter write paths for a scope.

use bytes::Bytes;

use crate::error::Result;

use crate::scope::Scope;

impl Scope {
    /// Apply a merge operation to one key.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the write fails.
    pub async fn merge<K: AsRef<[u8]>>(&self, key: K, op: storkv_core::MergeOp) -> Result<()> {
        self.activate_cache().await?;
        let key = key.as_ref();
        let connection = self.pool.acquire().await?;
        connection.merge(self, key, &op).await?;
        self.cache.invalidate(key);
        Ok(())
    }

    /// Atomically add `delta` to an integer counter at `key`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the write fails.
    pub async fn counter_add<K: AsRef<[u8]>>(&self, key: K, delta: i64) -> Result<()> {
        self.merge(key, storkv_core::MergeOp::CounterAdd(delta))
            .await
    }

    /// Append `data` to the value at `key`, keeping at most `max_len` trailing bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the write fails.
    pub async fn append_bytes<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        key: K,
        data: V,
        max_len: Option<usize>,
    ) -> Result<()> {
        self.merge(
            key,
            storkv_core::MergeOp::AppendBytes {
                data: Bytes::copy_from_slice(data.as_ref()),
                max_len,
            },
        )
        .await
    }

    /// Update `key` to the maximum of its current integer value and `value`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the write fails.
    pub async fn max_i64<K: AsRef<[u8]>>(&self, key: K, value: i64) -> Result<()> {
        self.merge(key, storkv_core::MergeOp::MaxI64(value)).await
    }

    /// Update `key` to the minimum of its current integer value and `value`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the write fails.
    pub async fn min_i64<K: AsRef<[u8]>>(&self, key: K, value: i64) -> Result<()> {
        self.merge(key, storkv_core::MergeOp::MinI64(value)).await
    }

    /// Apply a bitwise OR with `bits` to the `u64` value at `key`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the write fails.
    pub async fn bit_or<K: AsRef<[u8]>>(&self, key: K, bits: u64) -> Result<()> {
        self.merge(key, storkv_core::MergeOp::BitOr(bits)).await
    }

    /// Apply a bitwise AND with `bits` to the `u64` value at `key`.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the write fails.
    pub async fn bit_and<K: AsRef<[u8]>>(&self, key: K, bits: u64) -> Result<()> {
        self.merge(key, storkv_core::MergeOp::BitAnd(bits)).await
    }
}
