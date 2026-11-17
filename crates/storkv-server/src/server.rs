use std::path::PathBuf;
use std::sync::Arc;

use bytes::BytesMut;
use storkv_core::{Db, DbOptions, StorKvError};
use storkv_proto::{Message, MessagePayload, Request, Response, WireChangeEvent};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::listener::ServerListener;
use crate::metrics::ServerStats;
use crate::transports::{ConnectionGuard, TransportKind};

const READ_BUFFER_CAPACITY: usize = 8192;
const WATCH_CHANNEL_CAPACITY: usize = 128;

use std::sync::atomic::{AtomicBool, Ordering};

/// One network server bound to a single database.
pub struct Server {
    pub(crate) db: Arc<Db>,
    pub(crate) shutdown_sender: broadcast::Sender<()>,
    pub(crate) stats: Arc<ServerStats>,
    pub(crate) strict_admin: AtomicBool,
}

impl Server {
    /// Open the database at `data_dir` and wrap it in a server.
    ///
    /// # Errors
    ///
    /// Returns an error when the database cannot be opened.
    pub fn new(data_dir: PathBuf) -> Result<Self, StorKvError> {
        let db = Db::open(&data_dir, DbOptions::default())?;
        Ok(Self::with_db(Arc::new(db)))
    }

    /// Wrap an existing database handle in a server.
    pub fn with_db(db: Arc<Db>) -> Self {
        let (shutdown_sender, _) = broadcast::channel(1);
        Self {
            db,
            shutdown_sender,
            stats: Arc::new(ServerStats::default()),
            strict_admin: AtomicBool::new(false),
        }
    }

    /// Builder-style strict-admin toggle. When enabled, admin-only requests are
    /// rejected with [`storkv_proto::StorKvWireError::Unauthorized`] unless the
    /// connection was accepted on an admin listener or presented a valid token.
    pub fn with_strict_admin(self, strict_admin: bool) -> Self {
        self.strict_admin.store(strict_admin, Ordering::Relaxed);
        self
    }

    /// Change the strict-admin flag after construction.
    pub fn set_strict_admin(&self, strict_admin: bool) {
        self.strict_admin.store(strict_admin, Ordering::Relaxed);
    }

    /// Return the current strict-admin flag.
    pub fn strict_admin(&self) -> bool {
        self.strict_admin.load(Ordering::Relaxed)
    }

    /// Return the shared database handle.
    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    /// Return a sender that stops every listener when used.
    pub fn shutdown_handle(&self) -> broadcast::Sender<()> {
        self.shutdown_sender.clone()
    }

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
    /// [`AcceptedStream`] enum dispatches `AsyncRead`/`AsyncWrite` to the
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
            tokio::sync::mpsc::channel::<Message>(WATCH_CHANNEL_CAPACITY);
        let mut session = ConnectionSession {
            server: self,
            writer,
            watch_sender,
            watch_task: None,
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

struct ConnectionSession<W> {
    server: Arc<Server>,
    writer: W,
    watch_sender: tokio::sync::mpsc::Sender<Message>,
    watch_task: Option<JoinHandle<()>>,
    is_admin: bool,
}

impl<W: AsyncWrite + Unpin> ConnectionSession<W> {
    async fn process_buffered_messages(
        &mut self,
        read_buffer: &mut BytesMut,
    ) -> Result<bool, std::io::Error> {
        loop {
            let message = match Message::decode(read_buffer) {
                Ok(Some(message)) => message,
                Ok(None) => return Ok(true),
                Err(wire_error) => {
                    let error_response = Message::new_response(0, Response::Error(wire_error));
                    self.write_message(error_response).await?;
                    return Ok(false);
                }
            };
            self.handle_message(message).await?;
        }
    }

    async fn handle_message(&mut self, message: Message) -> Result<(), std::io::Error> {
        let MessagePayload::Request(request) = message.payload else {
            return Ok(());
        };
        self.server
            .stats
            .record_transport_request(TransportKind::Native.as_u8());
        if request == Request::WatchSubscribe {
            self.start_watch(message.request_id).await?;
        } else {
            let response = self.server.dispatch_request(request, self.is_admin);
            self.write_message(Message::new_response(message.request_id, response))
                .await?;
        }
        Ok(())
    }

    async fn start_watch(&mut self, request_id: u64) -> Result<(), std::io::Error> {
        let watch_receiver = self.server.db.watch();
        let watch_sender = self.watch_sender.clone();
        self.watch_task = Some(tokio::task::spawn_blocking(move || {
            while let Ok(event) = watch_receiver.recv() {
                let Some(wire_event) = wire_change_event(event) else {
                    continue;
                };
                let response = Message::new_response(request_id, Response::WatchEvent(wire_event));
                if watch_sender.blocking_send(response).is_err() {
                    break;
                }
            }
        }));
        self.write_message(Message::new_response(request_id, Response::Empty))
            .await
    }

    async fn write_message(&mut self, message: Message) -> Result<(), std::io::Error> {
        self.writer.write_all(&message.encode()).await
    }

    fn stop_watch(&mut self) {
        if let Some(handle) = self.watch_task.take() {
            handle.abort();
        }
    }
}

fn wire_change_event(event: storkv_core::watch::ChangeEvent) -> Option<WireChangeEvent> {
    match event {
        storkv_core::watch::ChangeEvent::Put {
            scope_id,
            key,
            value,
            seq,
        } => Some(WireChangeEvent::Put {
            scope_id,
            key,
            value,
            seq,
        }),
        storkv_core::watch::ChangeEvent::Delete { scope_id, key, seq } => {
            Some(WireChangeEvent::Delete { scope_id, key, seq })
        }
        storkv_core::watch::ChangeEvent::DropScope { scope_id, seq } => {
            Some(WireChangeEvent::DropScope { scope_id, seq })
        }
        _ => None,
    }
}
