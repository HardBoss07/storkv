//! etcd `/v3/lease/*` handlers.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response as HttpResponse};
use serde_json::{Value, json};

use crate::compat::redis::keyspace::{Outcome, atomic};
use crate::transports::http::json::encode_base64;

use super::{
    CODE_INVALID_ARGUMENT, field_number, gateway_error, now_ms, require_primary, revision, scope,
    write_error,
};
use crate::transports::http::HttpState;

/// etcd gRPC status code for a missing resource.
const CODE_NOT_FOUND: i32 = 5;
const MILLIS_PER_SECOND: u64 = 1000;

pub(crate) async fn grant(State(state): State<HttpState>, Json(body): Json<Value>) -> HttpResponse {
    if let Err(response) = require_primary(&state) {
        return *response;
    }
    let scope = match scope(&state) {
        Ok(scope) => scope,
        Err(response) => return *response,
    };
    let ttl_seconds = field_number(&body, "TTL").unwrap_or(0);
    if ttl_seconds == 0 {
        return gateway_error(
            StatusCode::BAD_REQUEST,
            CODE_INVALID_ARGUMENT,
            "lease TTL must be nonzero",
        );
    }
    let id = state
        .server
        .etcd_leases
        .grant(ttl_seconds.saturating_mul(MILLIS_PER_SECOND));
    revision(
        &state,
        scope.scope_id,
        json!({"ID": id.to_string(), "TTL": ttl_seconds.to_string()}),
    )
    .into_response()
}

pub(crate) async fn keepalive(
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
    let Some(id) = field_number(&body, "ID") else {
        return gateway_error(
            StatusCode::BAD_REQUEST,
            CODE_INVALID_ARGUMENT,
            "lease ID is required",
        );
    };
    let Some((granted_ttl_ms, keys)) = state.server.etcd_leases.snapshot(id, now_ms()) else {
        return lease_not_found();
    };
    let deadline_ms = now_ms().saturating_add(granted_ttl_ms);
    let result = atomic(state.server.db(), state.server.is_primary(), |atomic| {
        for key in &keys {
            let Some(value) = atomic.get(&scope, key)? else {
                continue;
            };
            atomic.put(&scope, key, &value, deadline_ms);
        }
        Ok(Outcome::Commit(()))
    });
    if let Err(error) = result {
        return write_error(error);
    }
    state.server.etcd_leases.refresh(id, deadline_ms);
    revision(
        &state,
        scope.scope_id,
        json!({
            "ID": id.to_string(),
            "TTL": (granted_ttl_ms / MILLIS_PER_SECOND).to_string(),
        }),
    )
    .into_response()
}

pub(crate) async fn revoke(
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
    let Some(id) = field_number(&body, "ID") else {
        return gateway_error(
            StatusCode::BAD_REQUEST,
            CODE_INVALID_ARGUMENT,
            "lease ID is required",
        );
    };
    let Some(keys) = state.server.etcd_leases.take(id) else {
        return lease_not_found();
    };
    let result = atomic(state.server.db(), state.server.is_primary(), |atomic| {
        for key in &keys {
            atomic.delete(&scope, key);
        }
        Ok(Outcome::Commit(()))
    });
    if let Err(error) = result {
        return write_error(error);
    }
    revision(&state, scope.scope_id, json!({})).into_response()
}

pub(crate) async fn time_to_live(
    State(state): State<HttpState>,
    Json(body): Json<Value>,
) -> HttpResponse {
    let scope = match scope(&state) {
        Ok(scope) => scope,
        Err(response) => return *response,
    };
    let Some(id) = field_number(&body, "ID") else {
        return gateway_error(
            StatusCode::BAD_REQUEST,
            CODE_INVALID_ARGUMENT,
            "lease ID is required",
        );
    };
    let include_keys = body.get("keys").and_then(Value::as_bool).unwrap_or(false);
    let now = now_ms();
    let Some((deadline_ms, granted_ttl_ms, keys)) = state.server.etcd_leases.status(id, now) else {
        return lease_not_found();
    };
    let remaining_ms = deadline_ms.saturating_sub(now);
    let mut payload = json!({
        "ID": id.to_string(),
        "TTL": remaining_ms.div_ceil(MILLIS_PER_SECOND).to_string(),
        "grantedTTL": (granted_ttl_ms / MILLIS_PER_SECOND).to_string(),
    });
    if include_keys {
        let encoded: Vec<String> = keys.iter().map(|key| encode_base64(key)).collect();
        if let Some(object) = payload.as_object_mut() {
            object.insert("keys".into(), Value::from(encoded));
        }
    }
    revision(&state, scope.scope_id, payload).into_response()
}

fn lease_not_found() -> HttpResponse {
    gateway_error(StatusCode::NOT_FOUND, CODE_NOT_FOUND, "lease not found")
}
