use bytes::BytesMut;
use futures_util::{SinkExt, StreamExt};
use storkv_proto::{Message, MessagePayload, Request, Response, WireChangeEvent};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::backend::decode_response;
use crate::transport::{ClientError, ClientResult};

/// WebSocket backend carrying native `STORKPRT` frames as binary messages.
pub(crate) struct WsBackend {
    socket: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    read_buf: BytesMut,
}

impl WsBackend {
    pub(crate) async fn connect(url: &str) -> ClientResult<Self> {
        let endpoint = websocket_endpoint(url);
        let (socket, _) = tokio_tungstenite::connect_async(&endpoint)
            .await
            .map_err(|error| ClientError::Http(error.to_string()))?;
        Ok(Self {
            socket,
            read_buf: BytesMut::with_capacity(8192),
        })
    }

    pub(crate) async fn send(
        &mut self,
        request_id: u64,
        request: Request,
    ) -> ClientResult<Response> {
        let encoded = Message::new_request(request_id, request).encode();
        self.socket
            .send(WsMessage::Binary(encoded))
            .await
            .map_err(|error| ClientError::Http(error.to_string()))?;
        loop {
            if let Some(response) = decode_response(&mut self.read_buf)? {
                return Ok(response);
            }
            let frame = self
                .socket
                .next()
                .await
                .ok_or_else(|| ClientError::Protocol("websocket closed".into()))?
                .map_err(|error| ClientError::Http(error.to_string()))?;
            match frame {
                WsMessage::Binary(bytes) => self.read_buf.extend_from_slice(&bytes),
                WsMessage::Text(text) => {
                    return Err(ClientError::Protocol(format!(
                        "unexpected text frame on binary websocket: {text}"
                    )));
                }
                WsMessage::Close(_) => {
                    return Err(ClientError::Protocol("websocket closed".into()));
                }
                _ => {}
            }
        }
    }

    pub(crate) async fn next_event(&mut self) -> ClientResult<Option<WireChangeEvent>> {
        loop {
            if let Some(message) = Message::decode(&mut self.read_buf)? {
                if let MessagePayload::Response(Response::WatchEvent(event)) = message.payload {
                    return Ok(Some(event));
                }
            }
            let frame = self
                .socket
                .next()
                .await
                .ok_or_else(|| ClientError::Protocol("websocket closed".into()))?
                .map_err(|error| ClientError::Http(error.to_string()))?;
            match frame {
                WsMessage::Binary(bytes) => self.read_buf.extend_from_slice(&bytes),
                WsMessage::Close(_) => return Ok(None),
                _ => {}
            }
        }
    }
}

/// Append the WebSocket upgrade path when the URI carries only an authority.
fn websocket_endpoint(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    let authority = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    if authority.contains('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1/ws")
    }
}
