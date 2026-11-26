//! HTTP REST, JSON-RPC, and WebSocket handlers built on axum.
//!
//! [`router`] mounts the `/v1/*` REST routes, the `/v1/rpc` JSON-RPC endpoint,
//! and the etcd-compatible `/v3/kv/*` routes.

use std::sync::Arc;

use axum::Router;
use axum::http::HeaderMap;
use axum::routing::{delete, get, post};

use storkv_proto::Request;

use crate::server::Server;
use crate::transports::{TransportKind, TransportOptions};

pub mod json;
pub mod rest;
mod rest_admin;
mod rest_data;
mod rest_scopes;
pub mod rpc;

#[derive(Clone)]
/// Shared state for HTTP and WebSocket handlers.
pub struct HttpState {
    pub(crate) server: Arc<Server>,
    pub(crate) options: TransportOptions,
}

impl HttpState {
    pub(crate) fn is_admin(&self, headers: &HeaderMap) -> bool {
        let header_token = headers
            .get("x-storkv-admin-token")
            .and_then(|value| value.to_str().ok());
        let bearer_token = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        self.options.grants_admin(header_token.or(bearer_token))
    }

    pub(crate) fn dispatch(&self, request: Request, headers: &HeaderMap) -> storkv_proto::Response {
        self.server
            .stats
            .record_transport_request(TransportKind::Http.as_u8());
        let consistency = crate::server::ConnectionConsistency::new(self.server.consistency());
        self.server
            .dispatch_request(request, self.is_admin(headers), &consistency)
    }
}

/// Build the HTTP router for the REST and JSON-RPC endpoints.
pub fn router(server: Arc<Server>, options: TransportOptions) -> Router {
    let state = HttpState { server, options };
    Router::new()
        .route("/v1/health", get(rest::health))
        .route("/v1/ping", get(rest::ping))
        .route("/v1/telemetry", get(rest::telemetry))
        .route("/v1/admin/stats", get(rest::admin_stats))
        .route("/v1/rpc", post(rpc::handle))
        .route("/v1/scopes", post(rest::create_scope))
        .route("/v1/scopes/{scope}", delete(rest::drop_scope))
        .route(
            "/v1/scopes/{scope}/keys/{key}",
            get(rest::get_key)
                .put(rest::put_key)
                .delete(rest::delete_key),
        )
        .route("/v1/scopes/{scope}/merge/{key}", post(rest::merge))
        .route("/v1/scopes/{scope}/delete-range", post(rest::delete_range))
        .route("/v1/scopes/{scope}/scan", get(rest::scan))
        .route("/v1/tx/begin", post(rest::tx_begin))
        .route("/v1/tx/commit", post(rest::tx_commit))
        .route("/v1/admin/checkpoint", post(rest::checkpoint))
        .route("/v1/admin/compact", post(rest::compact))
        .route("/v1/admin/backup", post(rest::trigger_backup))
        .route("/v1/admin/snapshots", post(rest::create_snapshot))
        .route("/v1/admin/snapshots/{name}", delete(rest::unpin_snapshot))
        .route("/v1/admin/verify-backup", post(rest::verify_backup))
        .route("/v1/admin/backups", get(rest::list_backups))
        .route("/v1/ws", get(crate::transports::ws::upgrade))
        .route("/v3/kv/put", post(crate::compat::etcd_http::put))
        .route("/v3/kv/range", post(crate::compat::etcd_http::range))
        .route(
            "/v3/kv/deleterange",
            post(crate::compat::etcd_http::delete_range),
        )
        .route("/v3/kv/txn", post(crate::compat::etcd_http::txn))
        .route(
            "/v3/lease/grant",
            post(crate::compat::etcd_http::lease_grant),
        )
        .route(
            "/v3/lease/keepalive",
            post(crate::compat::etcd_http::lease_keepalive),
        )
        .route(
            "/v3/lease/revoke",
            post(crate::compat::etcd_http::lease_revoke),
        )
        .route(
            "/v3/lease/timetolive",
            post(crate::compat::etcd_http::lease_time_to_live),
        )
        .with_state(state)
}
