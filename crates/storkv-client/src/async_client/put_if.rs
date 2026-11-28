//! Conditional one-round-trip writes for the asynchronous client.
//!
//! The server evaluates the condition and the write as one operation.

use std::time::Duration;

use bytes::Bytes;
use storkv_proto::{Request, Response, WirePutCondition, WirePutIfResult};

use crate::transport::{ClientResult, ScopeHandle};

use super::{AsyncClient, unexpected_response};

impl AsyncClient {
    /// Conditionally store one key/value pair in one round trip.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unknown or the request fails.
    pub async fn put_if<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        scope: &ScopeHandle,
        key: K,
        condition: &WirePutCondition,
        val: V,
        ttl: Option<Duration>,
    ) -> ClientResult<WirePutIfResult> {
        match self
            .send_request(Request::PutIf {
                scope_id: scope.scope_id,
                key: Bytes::copy_from_slice(key.as_ref()),
                condition: condition.clone(),
                val: Bytes::copy_from_slice(val.as_ref()),
                ttl,
            })
            .await?
        {
            Response::PutIf(result) => Ok(result),
            other => Err(unexpected_response(other)),
        }
    }
}
