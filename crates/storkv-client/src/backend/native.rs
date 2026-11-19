use bytes::BytesMut;
use storkv_proto::{Message, MessagePayload, Request, Response, WireChangeEvent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::backend::decode_response;
use crate::transport::{ClientError, ClientResult, ClientStream};

const READ_CHUNK_SIZE: usize = 8192;

/// Native `STORKPRT` v1 backend over TCP or a Unix-domain socket.
pub(crate) struct NativeBackend {
    stream: ClientStream,
    read_buf: BytesMut,
}

impl NativeBackend {
    pub(crate) async fn connect_tcp(address: &str) -> ClientResult<Self> {
        let stream = tokio::net::TcpStream::connect(address).await?;
        Ok(Self::new(ClientStream::Tcp(stream)))
    }

    pub(crate) async fn connect_unix(path: &str) -> ClientResult<Self> {
        #[cfg(unix)]
        {
            let stream = tokio::net::UnixStream::connect(path).await?;
            Ok(Self::new(ClientStream::Unix(stream)))
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(ClientError::Unsupported(
                "Unix-domain sockets are not supported on this platform".into(),
            ))
        }
    }

    fn new(stream: ClientStream) -> Self {
        Self {
            stream,
            read_buf: BytesMut::with_capacity(READ_CHUNK_SIZE),
        }
    }

    pub(crate) async fn send(
        &mut self,
        request_id: u64,
        request: Request,
    ) -> ClientResult<Response> {
        let encoded = Message::new_request(request_id, request).encode();
        self.stream.write_all(&encoded).await?;
        loop {
            if let Some(response) = decode_response(&mut self.read_buf)? {
                return Ok(response);
            }
            let mut chunk = [0u8; READ_CHUNK_SIZE];
            let bytes_read = self.stream.read(&mut chunk).await?;
            if bytes_read == 0 {
                return Err(ClientError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "Connection closed by server",
                )));
            }
            self.read_buf.extend_from_slice(&chunk[..bytes_read]);
        }
    }

    pub(crate) async fn next_event(&mut self) -> ClientResult<Option<WireChangeEvent>> {
        loop {
            if let Some(message) = Message::decode(&mut self.read_buf)? {
                if let MessagePayload::Response(Response::WatchEvent(event)) = message.payload {
                    return Ok(Some(event));
                }
            }
            let mut chunk = [0u8; READ_CHUNK_SIZE];
            let bytes_read = self.stream.read(&mut chunk).await?;
            if bytes_read == 0 {
                return Ok(None);
            }
            self.read_buf.extend_from_slice(&chunk[..bytes_read]);
        }
    }

    pub(crate) async fn next_replication(&mut self) -> ClientResult<Option<Response>> {
        loop {
            if let Some(message) = Message::decode(&mut self.read_buf)? {
                if let MessagePayload::Response(response) = message.payload {
                    return Ok(Some(response));
                }
            }
            let mut chunk = [0u8; READ_CHUNK_SIZE];
            let bytes_read = self.stream.read(&mut chunk).await?;
            if bytes_read == 0 {
                return Ok(None);
            }
            self.read_buf.extend_from_slice(&chunk[..bytes_read]);
        }
    }
}
