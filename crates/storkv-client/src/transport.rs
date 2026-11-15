use std::pin::Pin;
use std::task::{Context, Poll};

use storkv_proto::StorKvWireError;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// Connection transport supporting both TCP and Unix-domain sockets.
pub enum ClientStream {
    Tcp(TcpStream),
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

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("Server returned error: {0}")]
    Server(#[from] StorKvWireError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Unexpected response from server: {0}")]
    Protocol(String),
}

pub type ClientResult<T> = Result<T, ClientError>;

#[derive(Debug, Clone)]
pub struct ScopeHandle {
    pub scope_id: u32,
    pub name: String,
}
