//! Client error, result, and scope handle types.
use std::pin::Pin;
use std::task::{Context, Poll};

use storkv_proto::StorkvWireError;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// Connection transport supporting both TCP and Unix-domain sockets.
pub enum ClientStream {
    /// TCP connection.
    Tcp(TcpStream),
    /// Unix-domain socket connection.
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
}

impl AsyncRead for ClientStream {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        read_buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            ClientStream::Tcp(stream) => Pin::new(stream).poll_read(context, read_buffer),
            #[cfg(unix)]
            ClientStream::Unix(stream) => Pin::new(stream).poll_read(context, read_buffer),
        }
    }
}

impl AsyncWrite for ClientStream {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        write_buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            ClientStream::Tcp(stream) => Pin::new(stream).poll_write(context, write_buffer),
            #[cfg(unix)]
            ClientStream::Unix(stream) => Pin::new(stream).poll_write(context, write_buffer),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            ClientStream::Tcp(stream) => Pin::new(stream).poll_flush(context),
            #[cfg(unix)]
            ClientStream::Unix(stream) => Pin::new(stream).poll_flush(context),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            ClientStream::Tcp(stream) => Pin::new(stream).poll_shutdown(context),
            #[cfg(unix)]
            ClientStream::Unix(stream) => Pin::new(stream).poll_shutdown(context),
        }
    }
}

/// Client-side error type.
#[derive(Error, Debug)]
pub enum ClientError {
    /// The server returned a protocol error.
    #[error("Server returned error: {0}")]
    Server(#[from] StorkvWireError),
    /// An input or output operation failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// The server returned an unexpected message.
    #[error("Unexpected response from server: {0}")]
    Protocol(String),
    /// The transport cannot carry the operation.
    #[error("Transport does not support this operation: {0}")]
    Unsupported(String),
    /// An HTTP transport operation failed.
    #[error("HTTP transport error: {0}")]
    Http(String),
    /// A gRPC transport operation failed.
    #[error("gRPC transport error: {0}")]
    Grpc(String),
    /// A gRPC connection could not be established.
    #[error("gRPC connection error: {0}")]
    GrpcConnect(#[from] tonic::transport::Error),
    /// A gRPC call returned a non-OK status.
    #[error("gRPC status: {0}")]
    GrpcStatus(#[from] tonic::Status),
}

/// Result alias for client operations.
pub type ClientResult<T> = Result<T, ClientError>;

/// Identifier and name of a scope on the client side.
#[derive(Debug, Clone)]
pub struct ScopeHandle {
    /// Opaque scope identifier.
    pub scope_id: u32,
    /// Unique scope name.
    pub name: String,
}
