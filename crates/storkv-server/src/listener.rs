use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpListener;

/// Transport-agnostic listener used by [`crate::Server::run_listener`].
pub enum ServerListener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(tokio::net::UnixListener),
}

impl ServerListener {
    pub(crate) async fn accept(&self) -> std::io::Result<AcceptedStream> {
        match self {
            ServerListener::Tcp(listener) => listener
                .accept()
                .await
                .map(|(stream, _)| AcceptedStream::Tcp(stream)),
            #[cfg(unix)]
            ServerListener::Unix(listener) => listener
                .accept()
                .await
                .map(|(stream, _)| AcceptedStream::Unix(stream)),
        }
    }
}

/// A connection accepted from either a TCP listener or a Unix-domain socket.
pub enum AcceptedStream {
    Tcp(tokio::net::TcpStream),
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
}

impl AsyncRead for AcceptedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        read_buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            AcceptedStream::Tcp(stream) => Pin::new(stream).poll_read(context, read_buffer),
            #[cfg(unix)]
            AcceptedStream::Unix(stream) => Pin::new(stream).poll_read(context, read_buffer),
        }
    }
}

impl AsyncWrite for AcceptedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        write_buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            AcceptedStream::Tcp(stream) => Pin::new(stream).poll_write(context, write_buffer),
            #[cfg(unix)]
            AcceptedStream::Unix(stream) => Pin::new(stream).poll_write(context, write_buffer),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            AcceptedStream::Tcp(stream) => Pin::new(stream).poll_flush(context),
            #[cfg(unix)]
            AcceptedStream::Unix(stream) => Pin::new(stream).poll_flush(context),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            AcceptedStream::Tcp(stream) => Pin::new(stream).poll_shutdown(context),
            #[cfg(unix)]
            AcceptedStream::Unix(stream) => Pin::new(stream).poll_shutdown(context),
        }
    }
}
