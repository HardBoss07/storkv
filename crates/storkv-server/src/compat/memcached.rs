//! Memcached ASCII listener and per-connection session loop.

use std::sync::Arc;

use bytes::{Buf, Bytes, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::server::Server;
use crate::transports::{ConnectionGuard, TransportKind};

mod commands;
mod parser;

use commands::MemcachedSession;

const READ_CHUNK: usize = 16 * 1024;
const MAX_COMMAND_LINE: usize = 8 * 1024;

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
    let _guard = ConnectionGuard::new(server.clone(), TransportKind::Memcached);
    let (mut reader, mut writer) = stream.into_split();
    let mut buffer = BytesMut::with_capacity(READ_CHUNK);
    let mut chunk = [0u8; READ_CHUNK];
    let mut session = MemcachedSession::new(&server);
    let mut pending: Option<parser::Command> = None;

    loop {
        let bytes_read = reader.read(&mut chunk).await?;
        if bytes_read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..bytes_read]);

        loop {
            let (command, data) = if let Some(command) = pending.take() {
                let length = match command.data_len() {
                    Some(length) => length,
                    None => {
                        let response = session.execute(command, None);
                        if !response.is_empty() {
                            writer.write_all(&response).await?;
                        }
                        continue;
                    }
                };
                if buffer.len() < length + 2 {
                    pending = Some(command);
                    break;
                }
                if &buffer[length..length + 2] != b"\r\n" {
                    writer.write_all(b"CLIENT_ERROR bad data chunk\r\n").await?;
                    return Ok(());
                }
                let data = Bytes::copy_from_slice(&buffer[..length]);
                buffer.advance(length + 2);
                (command, Some(data))
            } else {
                let Some(line_end) = find_crlf(&buffer) else {
                    if buffer.len() > MAX_COMMAND_LINE {
                        writer
                            .write_all(b"CLIENT_ERROR command line too long\r\n")
                            .await?;
                        return Ok(());
                    }
                    break;
                };
                let line = Bytes::copy_from_slice(&buffer[..line_end]);
                buffer.advance(line_end + 2);
                let command = match parser::parse_line(&line) {
                    Ok(command) => command,
                    Err(error) if error.message == "unknown command" => {
                        writer.write_all(b"ERROR\r\n").await?;
                        continue;
                    }
                    Err(error) => {
                        let response = format!("CLIENT_ERROR {}\r\n", error.message);
                        writer.write_all(response.as_bytes()).await?;
                        continue;
                    }
                };
                if command.data_len().is_some() {
                    pending = Some(command);
                    continue;
                }
                (command, None)
            };
            let response = session.execute(command, data);
            if !response.is_empty() {
                writer.write_all(&response).await?;
            }
            if session.close {
                return Ok(());
            }
        }
    }
}

fn find_crlf(buffer: &[u8]) -> Option<usize> {
    let mut index = 0;
    while index + 1 < buffer.len() {
        if buffer[index] == b'\r' && buffer[index + 1] == b'\n' {
            return Some(index);
        }
        index += 1;
    }
    None
}
