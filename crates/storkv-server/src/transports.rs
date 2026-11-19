//! Transport front ends for the native, HTTP, WebSocket, gRPC, Redis, and
//! Memcached protocols.
//!
//! Each front end converts its wire format into a [`storkv_proto::Request`] and
//! calls `Server::dispatch_request`. Per-transport counters are keyed
//! by [`TransportKind`].

use std::sync::Arc;

use tokio::net::TcpListener;

use crate::server::Server;

pub mod grpc;
pub mod http;
pub mod ws;

/// Identifier used by per-transport counters in `WireAdminStats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// Native StorKV frame protocol.
    Native = 0,
    /// HTTP REST and JSON transport.
    Http = 1,
    /// WebSocket transport.
    WebSocket = 2,
    /// gRPC transport.
    Grpc = 3,
    /// Redis-compatible RESP transport.
    Redis = 4,
    /// Memcached-compatible text transport.
    Memcached = 5,
}

impl TransportKind {
    /// Return the numeric identifier used by per-transport counters.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Access policy shared by the HTTP, WebSocket, and gRPC transports.
#[derive(Debug, Clone, Default)]
pub struct TransportOptions {
    /// Every connection on this listener is treated as admin (for example a
    /// dedicated `--http-admin` listener).
    pub admin: bool,
    /// Optional shared secret accepted in `x-storkv-admin-token` or as a
    /// bearer token / gRPC metadata value.
    pub admin_token: Option<String>,
}

impl TransportOptions {
    /// Build access options for an auxiliary transport.
    pub fn new(admin: bool, admin_token: Option<String>) -> Self {
        Self { admin, admin_token }
    }

    /// Returns whether a presented token grants admin privileges on this
    /// connection.
    pub fn grants_admin(&self, presented_token: Option<&str>) -> bool {
        if self.admin {
            return true;
        }
        match (&self.admin_token, presented_token) {
            (Some(expected), Some(presented)) => tokens_match(expected, presented),
            _ => false,
        }
    }
}

fn tokens_match(expected: &str, presented: &str) -> bool {
    let expected_bytes = expected.as_bytes();
    let presented_bytes = presented.as_bytes();
    if expected_bytes.len() != presented_bytes.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in expected_bytes.iter().zip(presented_bytes.iter()) {
        difference |= left ^ right;
    }
    difference == 0
}

/// RAII guard keeping the active-connection counter accurate.
pub(crate) struct ConnectionGuard {
    server: Arc<Server>,
    kind: TransportKind,
}

impl ConnectionGuard {
    pub(crate) fn new(server: Arc<Server>, kind: TransportKind) -> Self {
        server.stats.record_transport_connection(kind.as_u8());
        Self { server, kind }
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.server
            .stats
            .record_transport_disconnection(self.kind.as_u8());
    }
}

/// Bind and serve the HTTP REST/WebSocket application.
pub async fn serve_http_tcp(
    server: Arc<Server>,
    listener: TcpListener,
    strict: TransportOptions,
) -> std::io::Result<()> {
    let app = http::router(server.clone(), strict.clone());
    serve_app(server, listener, app, strict).await
}

/// Serve only the WebSocket upgrade endpoint on a dedicated listener.
pub async fn serve_ws_tcp(
    server: Arc<Server>,
    listener: TcpListener,
    strict: TransportOptions,
) -> std::io::Result<()> {
    let app = ws::standalone_router(server.clone(), strict.clone());
    serve_app(server, listener, app, strict).await
}

async fn serve_app(
    server: Arc<Server>,
    listener: TcpListener,
    app: axum::Router,
    _options: TransportOptions,
) -> std::io::Result<()> {
    let mut shutdown_receiver = server.shutdown_sender.subscribe();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_receiver.recv().await;
        })
        .await
}

/// Bind and serve the gRPC services on an already-bound TCP listener.
pub async fn serve_grpc_tcp(
    server: Arc<Server>,
    listener: TcpListener,
    options: TransportOptions,
) -> Result<(), tonic::transport::Error> {
    grpc::serve(server, listener, options).await
}
