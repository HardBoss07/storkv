//! etcd `/v3/kv/put`, `/v3/kv/range`, and `/v3/kv/deleterange` handlers.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response as HttpResponse};
use serde_json::{Value, json};

use crate::compat::redis::keyspace::put_absolute;

use super::{
    CODE_INTERNAL, CODE_INVALID_ARGUMENT, entry_json, field, field_number, gateway_error, now_ms,
    require_primary, revision, scan_range, scope, write_error,
};
use crate::transports::http::HttpState;

pub(crate) async fn put(State(state): State<HttpState>, Json(body): Json<Value>) -> HttpResponse {
    if let Err(response) = require_primary(&state) {
        return *response;
    }
    let scope = match scope(&state) {
        Ok(scope) => scope,
        Err(response) => return *response,
    };
    let lease = field_number(&body, "lease").unwrap_or(0);
    let expires_at_ms = if lease == 0 {
        0
    } else {
        match state.server.etcd_leases.expiry(lease, now_ms()) {
            Some(deadline) => deadline,
            None => {
                return gateway_error(
                    StatusCode::BAD_REQUEST,
                    CODE_INVALID_ARGUMENT,
                    "lease not found",
                );
            }
        }
    };
    let Some(key) = field(&body, "key") else {
        return gateway_error(
            StatusCode::BAD_REQUEST,
            CODE_INVALID_ARGUMENT,
            "key is required",
        );
    };
    let value = field(&body, "value").unwrap_or_default();
    if let Err(error) = put_absolute(
        state.server.db(),
        state.server.is_primary(),
        &scope,
        &key,
        &value,
        expires_at_ms,
    ) {
        return write_error(error);
    }
    state.server.etcd_leases.set_key_lease(&key, lease);
    revision(&state, scope.scope_id, json!({})).into_response()
}

pub(crate) async fn range(State(state): State<HttpState>, Json(body): Json<Value>) -> HttpResponse {
    let scope = match scope(&state) {
        Ok(scope) => scope,
        Err(response) => return *response,
    };
    let Some(key) = field(&body, "key") else {
        return gateway_error(
            StatusCode::BAD_REQUEST,
            CODE_INVALID_ARGUMENT,
            "key is required",
        );
    };
    let range_end = field(&body, "range_end").unwrap_or_default();
    let limit = field_number(&body, "limit")
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(0);
    let count_only = body
        .get("count_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let keys_only = body
        .get("keys_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let pairs = match scan_range(
        &state,
        &scope,
        &key,
        &range_end,
        if limit == 0 { None } else { Some(limit) },
        keys_only,
    ) {
        Ok(pairs) => pairs,
        Err(error) => {
            return gateway_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                CODE_INTERNAL,
                &error.to_string(),
            );
        }
    };
    let count = pairs.len();
    let mut kvs = Vec::new();
    if !count_only {
        for (key, value) in pairs {
            let meta = state.server.db().key_meta(&scope, &key).ok().flatten();
            let version = meta.map(|meta| meta.seq).unwrap_or(0);
            let lease = state.server.etcd_leases.lease_for(&key);
            let value = (!keys_only).then_some(value.as_ref());
            kvs.push(entry_json(&key, value, version, lease));
        }
    }
    revision(
        &state,
        scope.scope_id,
        json!({"kvs": kvs, "count": count.to_string(), "more": false}),
    )
    .into_response()
}

pub(crate) async fn delete_range(
    State(state): State<HttpState>,
    Json(body): Json<Value>,
) -> HttpResponse {
    if let Err(response) = require_primary(&state) {
        return *response;
    }
    let scope = match scope(&state) {
        Ok(scope) => scope,
        Err(response) => return *response,
    };
    let Some(key) = field(&body, "key") else {
        return gateway_error(
            StatusCode::BAD_REQUEST,
            CODE_INVALID_ARGUMENT,
            "key is required",
        );
    };
    let range_end = field(&body, "range_end").unwrap_or_default();
    let pairs = match scan_range(&state, &scope, &key, &range_end, None, true) {
        Ok(pairs) => pairs,
        Err(error) => {
            return gateway_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                CODE_INTERNAL,
                &error.to_string(),
            );
        }
    };
    let deleted = pairs.len();
    let db = state.server.db();
    let mut transaction = db.begin_tx();
    for (key, _) in &pairs {
        if let Err(error) = transaction.delete(&scope, key) {
            return write_error(error);
        }
    }
    if let Err(error) = transaction.commit() {
        return write_error(error);
    }
    for (key, _) in &pairs {
        state.server.etcd_leases.detach(key);
    }
    revision(
        &state,
        scope.scope_id,
        json!({"deleted": deleted.to_string()}),
    )
    .into_response()
}
