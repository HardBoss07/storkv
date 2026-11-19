//! Connection accept loops for the native protocol.

use std::sync::Arc;

use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::session::ConnectionSession;
use super::{ConnectionConsistency, READ_BUFFER_CAPACITY, Server, WATCH_CHANNEL_CAPACITY};
use crate::listener::ServerListener;
use crate::transports::{ConnectionGuard, TransportKind};

impl Server {
    /// Accept native protocol connections from a TCP listener.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when accept fails.
    pub async fn run_tcp_listener(
        self: Arc<Self>,
        listener: TcpListener,
        is_admin: bool,
    ) -> Result<(), std::io::Error> {
        self.run_listener(ServerListener::Tcp(listener), is_admin)
            .await
    }

    /// Accept connections from either a TCP or a Unix-domain socket.
    ///
    /// A single accept loop/handler is shared by both transports; the
    /// `AcceptedStream` enum dispatches `AsyncRead`/`AsyncWrite` to the
    /// concrete stream type.
    pub async fn run_listener(
        self: Arc<Self>,
        listener: ServerListener,
        is_admin: bool,
    ) -> Result<(), std::io::Error> {
        let mut shutdown_receiver = self.shutdown_sender.subscribe();

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    let stream = accept_result?;
                    spawn_connection(self.clone(), stream, is_admin);
                }
                _ = shutdown_receiver.recv() => break,
            }
        }

        Ok(())
    }

    #[cfg(unix)]
    /// Accept native protocol connections from a Unix-domain socket listener.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when accept fails.
    pub async fn run_uds_listener(
        self: Arc<Self>,
        listener: tokio::net::UnixListener,
        is_admin: bool,
    ) -> Result<(), std::io::Error> {
        self.run_listener(ServerListener::Unix(listener), is_admin)
            .await
    }

    /// Handle one accepted connection until the peer closes or the server stops.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when a read or write fails.
    pub async fn handle_connection<Stream>(
        self: Arc<Self>,
        stream: Stream,
        is_admin: bool,
    ) -> Result<(), std::io::Error>
    where
        Stream: AsyncReadExt + AsyncWriteExt + Unpin + Send + 'static,
    {
        let mut read_buffer = BytesMut::with_capacity(READ_BUFFER_CAPACITY);
        let (mut reader, writer) = tokio::io::split(stream);
        let (watch_sender, mut watch_receiver) =
            tokio::sync::mpsc::channel::<storkv_proto::Message>(WATCH_CHANNEL_CAPACITY);
        let default_consistency = self.consistency();
        let mut session = ConnectionSession {
            server: self,
            writer,
            watch_sender,
            watch_task: None,
            replication_task: None,
            consistency: ConnectionConsistency::new(default_consistency),
            is_admin,
        };
        let mut read_chunk = [0u8; READ_BUFFER_CAPACITY];
        let mut shutdown_receiver = session.server.shutdown_sender.subscribe();

        loop {
            tokio::select! {
                read_result = reader.read(&mut read_chunk) => {
                    let bytes_read = read_result?;
                    if bytes_read == 0 {
                        break;
                    }
                    read_buffer.extend_from_slice(&read_chunk[..bytes_read]);
                    if !session.process_buffered_messages(&mut read_buffer).await? {
                        return Ok(());
                    }
                }
                Some(watch_message) = watch_receiver.recv() => {
                    session.write_message(watch_message).await?;
                }
                _ = shutdown_receiver.recv() => {
                    break;
                }
            }
        }

        session.stop_watch();
        Ok(())
    }
}

fn spawn_connection<Stream>(server: Arc<Server>, stream: Stream, is_admin: bool)
where
    Stream: AsyncReadExt + AsyncWriteExt + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let _guard = ConnectionGuard::new(server.clone(), TransportKind::Native);
        let _ = server.handle_connection(stream, is_admin).await;
    });
}
