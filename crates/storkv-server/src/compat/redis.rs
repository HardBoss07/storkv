//! Redis RESP2/RESP3 listener and per-connection session loop.

use std::sync::Arc;

use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::server::Server;
use crate::transports::{ConnectionGuard, TransportKind};

mod codec;
mod codec_decode;
mod codec_encode;
mod commands;
mod commands_client;
mod commands_keys;
mod commands_server;
mod commands_strings;
mod commands_tx;
mod helpers;
pub(crate) mod keyspace;

use codec::Resp;
use codec_decode::{MAX_READ_BUFFER, into_arguments, parse};
use codec_encode::encode;
use commands::RedisSession;

const READ_CHUNK: usize = 16 * 1024;

pub(crate) async fn serve(server: Arc<Server>, listener: TcpListener) -> std::io::Result<()> {
    let mut shutdown = server.shutdown_handle().subscribe();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let session_server = server.clone();
                tokio::spawn(async move {
                    let _ = run_session(session_server, stream).await;
                });
            }
            _ = shutdown.recv() => break,
        }
    }
    Ok(())
}

async fn run_session(server: Arc<Server>, stream: TcpStream) -> std::io::Result<()> {
    let _guard = ConnectionGuard::new(server.clone(), TransportKind::Redis);
    let (mut reader, mut writer) = stream.into_split();
    let mut buffer = BytesMut::with_capacity(READ_CHUNK);
    let mut chunk = [0u8; READ_CHUNK];
    let mut session = RedisSession::new(server);

    loop {
        let bytes_read = reader.read(&mut chunk).await?;
        if bytes_read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..bytes_read]);
        loop {
            match parse(&mut buffer) {
                Ok(Some(value)) => {
                    let arguments = match into_arguments(value) {
                        Ok(arguments) => arguments,
                        Err(error) => {
                            write_error(&mut writer, session.resp_version(), &error.to_string())
                                .await?;
                            return Ok(());
                        }
                    };
                    let response = session.execute(arguments);
                    let mut encoded = BytesMut::new();
                    encode(&response, session.resp_version(), &mut encoded);
                    writer.write_all(&encoded).await?;
                    if session.close {
                        return Ok(());
                    }
                }
                Ok(None) => {
                    if buffer.len() > MAX_READ_BUFFER {
                        write_error(
                            &mut writer,
                            session.resp_version(),
                            "request exceeds maximum buffer size",
                        )
                        .await?;
                        return Ok(());
                    }
                    break;
                }
                Err(error) => {
                    write_error(&mut writer, session.resp_version(), &error.to_string()).await?;
                    return Ok(());
                }
            }
        }
    }
}

async fn write_error<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    version: u8,
    message: &str,
) -> std::io::Result<()> {
    let mut encoded = BytesMut::new();
    encode(
        &Resp::error(format!("ERR Protocol error: {message}")),
        version,
        &mut encoded,
    );
    writer.write_all(&encoded).await
}
