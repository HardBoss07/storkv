use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use bytes::Bytes;
use storkv_proto::{Request, Response, WireMergeOp, WireTxMutation};

use crate::async_client::AsyncClient;
use crate::transport::{ClientError, ClientResult, ScopeHandle};

type ReadSet = HashSet<(u32, Bytes)>;
type WriteSet = BTreeMap<(u32, Bytes), WireTxMutation>;

/// Client-side transaction with read-your-own-writes and OCC commit.
pub struct AsyncTransaction {
    read_seq: u64,
    client: AsyncClient,
    read_set: ReadSet,
    write_set: WriteSet,
}

impl AsyncTransaction {
    pub(crate) fn new(client: AsyncClient, read_seq: u64) -> Self {
        Self {
            client,
            read_seq,
            read_set: HashSet::new(),
            write_set: BTreeMap::new(),
        }
    }

    /// Read one key and add the key to the OCC read set.
    pub async fn get<K: AsRef<[u8]>>(
        &mut self,
        scope: &ScopeHandle,
        key: K,
    ) -> ClientResult<Option<Bytes>> {
        let key_bytes = Bytes::copy_from_slice(key.as_ref());
        self.read_set.insert((scope.scope_id, key_bytes.clone()));

        if let Some(mutation) = self.write_set.get(&(scope.scope_id, key_bytes)).cloned() {
            return self.read_own_write(scope, key.as_ref(), mutation).await;
        }

        self.client.get(scope, key).await
    }

    /// Read an ordered key range and add every returned key to the OCC read set.
    pub async fn scan<K: AsRef<[u8]>>(
        &mut self,
        scope: &ScopeHandle,
        start_key: Option<K>,
        end_key: Option<K>,
    ) -> ClientResult<Vec<(Bytes, Bytes)>> {
        let entries = self.client.scan(scope, start_key, end_key).await?;
        self.record_scan(scope, &entries);
        Ok(entries)
    }

    /// Read every key with `prefix` and add every returned key to the OCC read set.
    pub async fn scan_prefix<K: AsRef<[u8]>>(
        &mut self,
        scope: &ScopeHandle,
        prefix: K,
    ) -> ClientResult<Vec<(Bytes, Bytes)>> {
        let entries = self.client.scan_prefix(scope, prefix).await?;
        self.record_scan(scope, &entries);
        Ok(entries)
    }

    fn record_scan(&mut self, scope: &ScopeHandle, entries: &[(Bytes, Bytes)]) {
        for (key, _) in entries {
            self.read_set.insert((scope.scope_id, key.clone()));
        }
    }

    /// Read-your-own-writes: inspect the pending write set before the server.
    async fn read_own_write(
        &self,
        scope: &ScopeHandle,
        key: &[u8],
        mutation: WireTxMutation,
    ) -> ClientResult<Option<Bytes>> {
        match mutation {
            WireTxMutation::Put { val, .. } => Ok(Some(val)),
            WireTxMutation::Delete => Ok(None),
            WireTxMutation::Merge { operand } => {
                let base_value = self.client.get(scope, key).await?;
                Ok(Some(merge_counter_value(base_value, &operand)))
            }
            WireTxMutation::DeleteRange { .. } => Ok(None),
            _ => Err(ClientError::Protocol(
                "unsupported transaction mutation variant".into(),
            )),
        }
    }

    /// Buffer a value write. `0` expiry means no expiry.
    pub fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(&mut self, scope: &ScopeHandle, key: K, val: V) {
        self.insert_mutation(
            scope,
            key,
            WireTxMutation::Put {
                val: Bytes::copy_from_slice(val.as_ref()),
                expires_at_ms: 0,
            },
        );
    }

    /// Buffer a value write with a time to live.
    pub fn put_with_ttl<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &mut self,
        scope: &ScopeHandle,
        key: K,
        val: V,
        ttl: Duration,
    ) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.insert_mutation(
            scope,
            key,
            WireTxMutation::Put {
                val: Bytes::copy_from_slice(val.as_ref()),
                expires_at_ms: now_ms + ttl.as_millis() as u64,
            },
        );
    }

    /// Buffer a delete.
    pub fn delete<K: AsRef<[u8]>>(&mut self, scope: &ScopeHandle, key: K) {
        self.insert_mutation(scope, key, WireTxMutation::Delete);
    }

    /// Buffer a half-open range delete.
    pub fn delete_range<K: AsRef<[u8]>>(&mut self, scope: &ScopeHandle, start_key: K, end_key: K) {
        self.insert_mutation(
            scope,
            start_key,
            WireTxMutation::DeleteRange {
                end_key: Bytes::copy_from_slice(end_key.as_ref()),
            },
        );
    }

    /// Buffer a merge operand.
    pub fn merge<K: AsRef<[u8]>>(&mut self, scope: &ScopeHandle, key: K, op: WireMergeOp) {
        let operand = match op {
            WireMergeOp::CounterAdd(delta) => Bytes::from(delta.to_string()),
            WireMergeOp::Custom(operand) => operand,
            _ => return,
        };
        self.insert_mutation(scope, key, WireTxMutation::Merge { operand });
    }

    /// Send the write set and read set to the server.
    pub async fn commit(self) -> ClientResult<()> {
        let read_set = self.read_set.into_iter().collect();
        let write_set = self.write_set.into_iter().collect();

        match self
            .client
            .send_request(Request::TxCommit {
                read_seq: self.read_seq,
                read_set,
                write_set,
            })
            .await?
        {
            Response::Empty => Ok(()),
            other => Err(ClientError::Protocol(format!(
                "Unexpected response: {:?}",
                other
            ))),
        }
    }

    fn insert_mutation<K: AsRef<[u8]>>(
        &mut self,
        scope: &ScopeHandle,
        key: K,
        mutation: WireTxMutation,
    ) {
        let scoped_key = (scope.scope_id, Bytes::copy_from_slice(key.as_ref()));
        self.write_set.insert(scoped_key, mutation);
    }
}

fn merge_counter_value(base_value: Option<Bytes>, operand: &[u8]) -> Bytes {
    let mut total = 0i64;
    if let Some(base_bytes) = base_value
        && let Ok(base_text) = std::str::from_utf8(&base_bytes)
    {
        total = base_text.parse::<i64>().unwrap_or(0);
    }
    if let Ok(operand_text) = std::str::from_utf8(operand) {
        total += operand_text.parse::<i64>().unwrap_or(0);
    }
    Bytes::from(total.to_string())
}
