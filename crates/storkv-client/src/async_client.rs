//! Asynchronous client core.
//!
//! One client owns one backend connection behind a mutex. Requests serialize
//! on that mutex. The request counter never reuses an identifier.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use storkv_proto::{Consistency, Request, Response};
use tokio::sync::Mutex;

use crate::backend::Backend;
use crate::transport::{ClientError, ClientResult};
use crate::uri::TransportKind;

mod admin;
mod data_ops;
mod put_if;
mod scope_ops;
mod streams;

pub use streams::{ReplicationStream, WatchStream};

/// Asynchronous client for the StorKV protocol.
#[derive(Clone)]
pub struct AsyncClient {
    backend: Arc<Mutex<Backend>>,
    request_counter: Arc<AtomicU64>,
}

impl AsyncClient {
    /// Connect over TCP (`host:port`).
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Io` when the TCP connection fails.
    pub async fn connect(address: &str) -> ClientResult<Self> {
        Self::from_backend(Backend::connect(TransportKind::NativeTcp, address).await?)
    }

    /// Connect to either a TCP endpoint or a Unix-domain socket path.
    ///
    /// Addresses with the `unix:` scheme or a filesystem-like path select a
    /// Unix-domain socket. Other addresses use TCP.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Protocol` when the URI is invalid.
    /// Returns `ClientError::Unsupported` when the platform has no Unix sockets.
    /// Returns `ClientError::Io` when the connection fails.
    pub async fn connect_any(address: &str) -> ClientResult<Self> {
        let (kind, target) = TransportKind::parse_uri(address)?;
        match kind {
            TransportKind::NativeTcp | TransportKind::NativeUnix => {
                Self::from_backend(Backend::connect(kind, &target).await?)
            }
            other => Self::from_backend(Backend::connect(other, &target).await?),
        }
    }

    /// Connect using any supported scheme: `storkv://`, `unix://`, `http://`,
    /// `ws://`, or `grpc://`. A bare `host:port` remains native TCP.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Protocol` when the URI is invalid.
    /// Returns `ClientError::Unsupported` when the platform has no Unix sockets.
    /// Returns `ClientError::Io`, `ClientError::Http`, `ClientError::Grpc`,
    /// or `ClientError::GrpcConnect` when the connection fails.
    pub async fn connect_uri(uri: &str) -> ClientResult<Self> {
        let (kind, target) = TransportKind::parse_uri(uri)?;
        Self::from_backend(Backend::connect(kind, &target).await?)
    }

    fn from_backend(backend: Backend) -> ClientResult<Self> {
        Ok(Self {
            backend: Arc::new(Mutex::new(backend)),
            request_counter: Arc::new(AtomicU64::new(1)),
        })
    }

    fn next_request_id(&self) -> u64 {
        self.request_counter.fetch_add(1, Ordering::SeqCst)
    }

    /// Send one request and receive its response.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Server` when the server replies with an error.
    /// Returns `ClientError::Protocol` when the reply is malformed.
    /// Returns `ClientError::Io`, `ClientError::Http`, `ClientError::Grpc`,
    /// or `ClientError::GrpcStatus` when the transport fails.
    pub async fn send_request(&self, request: Request) -> ClientResult<Response> {
        let request_id = self.next_request_id();
        let mut backend = self.backend.lock().await;
        backend.send(request_id, request).await
    }

    /// Send a ping and require a pong reply.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Protocol` when the reply is not a pong.
    /// Returns `ClientError::Io` or `ClientError::Http` when the send fails.
    pub async fn ping(&self) -> ClientResult<()> {
        expect_variant(self.send_request(Request::Ping).await?, Response::Pong)
    }

    /// Select the consistency level for later data operations on this server.
    ///
    /// # Errors
    ///
    /// Returns `ClientError::Protocol` when the reply is not an empty response.
    /// Returns `ClientError::Io` or `ClientError::Http` when the send fails.
    pub async fn set_consistency(&self, consistency: Consistency) -> ClientResult<()> {
        expect_variant(
            self.send_request(Request::ConfigureConsistency { consistency })
                .await?,
            Response::Empty,
        )
    }
}

pub(super) fn expect_empty(response: Response) -> ClientResult<()> {
    expect_variant(response, Response::Empty)
}

pub(super) fn expect_variant(response: Response, expected: Response) -> ClientResult<()> {
    if response == expected {
        Ok(())
    } else {
        Err(unexpected_response(response))
    }
}

pub(super) fn unexpected_response(response: Response) -> ClientError {
    ClientError::Protocol(format!("Unexpected response: {:?}", response))
}
