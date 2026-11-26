//! etcd `/v3/kv/txn` handler built on one `storkv-core` OCC transaction.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response as HttpResponse};
use bytes::Bytes;
use serde_json::{Value, json};
use storkv_core::{ScopeHandle, StorkvError, Transaction};

use super::{
    CODE_INTERNAL, RANGE_END_TO_LAST_KEY, entry_json, field, field_number, gateway_error, now_ms,
    require_primary, revision, scope, write_error,
};
use crate::transports::http::HttpState;

mod compare;

use compare::evaluate;

const MAX_RETRIES: usize = 16;

pub(crate) async fn txn(State(state): State<HttpState>, Json(body): Json<Value>) -> HttpResponse {
    if let Err(response) = require_primary(&state) {
        return *response;
    }
    let scope = match scope(&state) {
        Ok(scope) => scope,
        Err(response) => return *response,
    };
    let compares = operations(&body, "compare");
    let success = operations(&body, "success");
    let failure = operations(&body, "failure");
    let db = state.server.db();

    for _ in 0..MAX_RETRIES {
        let mut tx = db.begin_tx();
        let matched = match evaluate(&mut tx, &scope, &compares) {
            Ok(matched) => matched,
            Err(error) => return write_error(error),
        };
        let mut responses = Vec::new();
        let mut attachments = Vec::new();
        let operations = if matched { &success } else { &failure };
        for operation in operations {
            if let Err(error) = apply(
                &mut tx,
                &scope,
                &state,
                operation,
                &mut responses,
                &mut attachments,
            ) {
                return write_error(error);
            }
        }
        match tx.commit() {
            Ok(()) => {
                for (key, lease) in attachments {
                    state.server.etcd_leases.set_key_lease(&key, lease);
                }
                return revision(
                    &state,
                    scope.scope_id,
                    json!({"succeeded": matched, "responses": responses}),
                )
                .into_response();
            }
            Err(StorkvError::WriteConflict { .. }) => continue,
            Err(error) => return write_error(error),
        }
    }
    gateway_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        CODE_INTERNAL,
        "transaction conflict",
    )
}

fn operations(body: &Value, name: &str) -> Vec<Value> {
    body.get(name)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn apply(
    tx: &mut Transaction,
    scope: &ScopeHandle,
    state: &HttpState,
    operation: &Value,
    responses: &mut Vec<Value>,
    attachments: &mut Vec<(Vec<u8>, u64)>,
) -> Result<(), StorkvError> {
    if let Some(put) = operation.get("request_put") {
        let key = required_key(put)?;
        let value = field(put, "value").unwrap_or_default();
        let lease = field_number(put, "lease").unwrap_or(0);
        let expires_at_ms = if lease == 0 {
            0
        } else {
            state
                .server
                .etcd_leases
                .expiry(lease, now_ms())
                .ok_or_else(|| invalid("lease not found"))?
        };
        tx.put_with_expires_at(scope, &key, &value, expires_at_ms)?;
        attachments.push((key, lease));
        responses.push(json!({"response_put": {}}));
        return Ok(());
    }
    if let Some(range) = operation.get("request_range") {
        responses.push(json!({"response_range": range_response(tx, scope, range)?}));
        return Ok(());
    }
    if let Some(delete) = operation.get("request_delete_range") {
        responses.push(json!({"response_delete_range": delete_response(tx, scope, delete)?}));
        return Ok(());
    }
    Err(invalid("unsupported transaction operation"))
}

fn range_response(
    tx: &mut Transaction,
    scope: &ScopeHandle,
    spec: &Value,
) -> Result<Value, StorkvError> {
    let key = required_key(spec)?;
    let range_end = field(spec, "range_end").unwrap_or_default();
    let limit = field_number(spec, "limit")
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(0);
    let count_only = spec
        .get("count_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let keys_only = spec
        .get("keys_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut pairs = if range_end.is_empty() {
        tx.get(scope, &key)?
            .map(|value| vec![(Bytes::from(key), value)])
            .unwrap_or_default()
    } else {
        let end = (range_end != RANGE_END_TO_LAST_KEY).then_some(range_end.as_slice());
        tx.scan(scope, Some(key.as_slice()), end)?
    };
    if limit != 0 {
        pairs.truncate(limit);
    }
    let count = pairs.len();
    let mut kvs = Vec::new();
    if !count_only {
        for (key, value) in pairs {
            let version = tx.key_meta(scope, &key)?.map(|meta| meta.seq).unwrap_or(0);
            let value = (!keys_only).then_some(value.as_ref());
            kvs.push(entry_json(&key, value, version, 0));
        }
    }
    Ok(json!({"kvs": kvs, "count": count.to_string(), "more": false}))
}

fn delete_response(
    tx: &mut Transaction,
    scope: &ScopeHandle,
    spec: &Value,
) -> Result<Value, StorkvError> {
    let key = required_key(spec)?;
    let range_end = field(spec, "range_end").unwrap_or_default();
    let deleted = if range_end.is_empty() {
        let present = tx.get(scope, &key)?.is_some();
        tx.delete(scope, &key)?;
        usize::from(present)
    } else {
        let end = (range_end != RANGE_END_TO_LAST_KEY).then_some(range_end.as_slice());
        let keys: Vec<Bytes> = tx
            .scan(scope, Some(key.as_slice()), end)?
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        for key in &keys {
            tx.key_meta(scope, key)?;
        }
        tx.delete_range(scope, key.as_slice(), range_end.as_slice())?;
        keys.len()
    };
    Ok(json!({"deleted": deleted.to_string()}))
}

fn required_key(spec: &Value) -> Result<Vec<u8>, StorkvError> {
    field(spec, "key").ok_or_else(|| invalid("key is required"))
}

fn invalid(message: &str) -> StorkvError {
    StorkvError::InvalidOperation(message.to_string())
}
