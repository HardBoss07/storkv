use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::BytesMut;
use storkv_proto::{
    Message, MessagePayload, Request, Response, WireAdminStats, WireTelemetrySnapshot,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::transaction::AsyncTransaction;
use crate::transport::{ClientError, ClientResult, ClientStream};

mod scope_ops;

const READ_CHUNK_SIZE: usize = 8192;

#[derive(Clone)]
pub struct AsyncClient {
    stream: Arc<Mutex<ClientStream>>,
    read_buf: Arc<Mutex<BytesMut>>,
    request_counter: Arc<AtomicU64>,
}

impl AsyncClient {
    /// Connect over TCP (`host:port`).
    pub async fn connect(address: &str) -> ClientResult<Self> {
        let stream = TcpStream::connect(address).await?;
        Self::from_stream(ClientStream::Tcp(stream))
    }

    /// Connect to either a TCP endpoint or a Unix-domain socket path. Addresses
    /// using the `unix:` scheme or filesystem-like paths are treated as UDS.
    pub async fn connect_any(address: &str) -> ClientResult<Self> {
        if storkv_proto::is_unix_socket_addr(address) {
            #[cfg(unix)]
            {
                let socket_path = storkv_proto::unix_socket_path(address);
                let stream = tokio::net::UnixStream::connect(socket_path).await?;
                return Self::from_stream(ClientStream::Unix(stream));
            }
            #[cfg(not(unix))]
            {
                return Err(ClientError::Io(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "Unix-domain sockets are not supported on this platform",
                )));
            }
        }
        Self::connect(address).await
    }

    fn from_stream(stream: ClientStream) -> ClientResult<Self> {
        Ok(Self {
            stream: Arc::new(Mutex::new(stream)),
            read_buf: Arc::new(Mutex::new(BytesMut::with_capacity(READ_CHUNK_SIZE))),
            request_counter: Arc::new(AtomicU64::new(1)),
        })
    }

    fn next_request_id(&self) -> u64 {
        self.request_counter.fetch_add(1, Ordering::SeqCst)
    }

    pub async fn send_request(&self, request: Request) -> ClientResult<Response> {
        let request_id = self.next_request_id();
        let message = Message::new_request(request_id, request);
        let encoded_message = message.encode();

        let mut stream = self.stream.lock().await;
        stream.write_all(&encoded_message).await?;

        let mut read_buf = self.read_buf.lock().await;
        loop {
            if let Some(response) = decode_response(&mut read_buf)? {
                return Ok(response);
            }
            read_more(&mut stream, &mut read_buf).await?;
        }
    }

    pub async fn ping(&self) -> ClientResult<()> {
        expect_variant(self.send_request(Request::Ping).await?, Response::Pong)
    }

    pub async fn health_check(&self) -> ClientResult<()> {
        expect_variant(
            self.send_request(Request::HealthCheck).await?,
            Response::HealthOk,
        )
    }

    pub async fn telemetry(&self) -> ClientResult<WireTelemetrySnapshot> {
        match self.send_request(Request::TelemetryQuery).await? {
            Response::Telemetry(snapshot) => Ok(snapshot),
            other => Err(unexpected_response(other)),
        }
    }

    /// Fetch admin statistics (L0..L6 SSTable distribution, per-scope counters,
    /// and observed WAL commit latency percentile).
    pub async fn admin_stats(&self) -> ClientResult<WireAdminStats> {
        match self.send_request(Request::AdminStats).await? {
            Response::AdminStats(stats) => Ok(stats),
            other => Err(unexpected_response(other)),
        }
    }

    pub async fn begin_tx(&self) -> ClientResult<AsyncTransaction> {
        let read_sequence = match self.send_request(Request::TxBegin).await? {
            Response::TxStarted { read_seq } => read_seq,
            other => {
                return Err(ClientError::Protocol(format!(
                    "Unexpected response for TxBegin: {:?}",
                    other
                )));
            }
        };

        Ok(AsyncTransaction::new(self.clone(), read_sequence))
    }
}

fn decode_response(read_buf: &mut BytesMut) -> ClientResult<Option<Response>> {
    let Some(message) = Message::decode(read_buf)? else {
        return Ok(None);
    };
    match message.payload {
        MessagePayload::Response(Response::Error(error)) => Err(ClientError::Server(error)),
        MessagePayload::Response(response) => Ok(Some(response)),
        MessagePayload::Request(_) => Err(ClientError::Protocol(
            "Expected response from server, got request".into(),
        )),
    }
}

async fn read_more(stream: &mut ClientStream, read_buf: &mut BytesMut) -> ClientResult<()> {
    let mut chunk = [0u8; READ_CHUNK_SIZE];
    let bytes_read = stream.read(&mut chunk).await?;
    if bytes_read == 0 {
        return Err(ClientError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "Connection closed by server",
        )));
    }
    read_buf.extend_from_slice(&chunk[..bytes_read]);
    Ok(())
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
