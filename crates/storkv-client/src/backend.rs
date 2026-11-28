//! Backend transport selection and shared connection state.
use storkv_proto::{Message, MessagePayload, Request, Response};

use crate::transport::{ClientError, ClientResult};
use crate::uri::TransportKind;

mod grpc;
mod http;
mod native;
mod ws;

pub(crate) use grpc::GrpcBackend;
pub(crate) use http::HttpBackend;
pub(crate) use native::NativeBackend;
pub(crate) use ws::WsBackend;

/// A single connected transport backend owned by one [`crate::AsyncClient`].
pub(crate) enum Backend {
    Native(NativeBackend),
    Http(HttpBackend),
    Ws(WsBackend),
    Grpc(Box<GrpcBackend>),
}

impl Backend {
    pub(crate) async fn connect(kind: TransportKind, target: &str) -> ClientResult<Self> {
        match kind {
            TransportKind::NativeTcp => Ok(Self::Native(NativeBackend::connect_tcp(target).await?)),
            TransportKind::NativeUnix => {
                Ok(Self::Native(NativeBackend::connect_unix(target).await?))
            }
            TransportKind::Http => Ok(Self::Http(HttpBackend::connect(target).await?)),
            TransportKind::WebSocket => Ok(Self::Ws(WsBackend::connect(target).await?)),
            TransportKind::Grpc => Ok(Self::Grpc(Box::new(GrpcBackend::connect(target).await?))),
        }
    }

    pub(crate) async fn send(
        &mut self,
        request_id: u64,
        request: Request,
    ) -> ClientResult<Response> {
        match self {
            Backend::Native(native) => native.send(request_id, request).await,
            Backend::Http(http) => http.send(request_id, request).await,
            Backend::Ws(ws) => ws.send(request_id, request).await,
            Backend::Grpc(grpc) => grpc.send(request).await,
        }
    }

    pub(crate) async fn start_watch(
        &mut self,
        request_id: u64,
        from_seq: Option<u64>,
    ) -> ClientResult<()> {
        match self {
            Backend::Native(_) | Backend::Ws(_) => {
                let request = match from_seq {
                    Some(from_seq) => Request::WatchFrom { from_seq },
                    None => Request::WatchSubscribe,
                };
                let response = self.send(request_id, request).await?;
                if response == Response::Empty {
                    Ok(())
                } else {
                    Err(ClientError::Protocol(format!(
                        "Unexpected response: {response:?}"
                    )))
                }
            }
            Backend::Grpc(grpc) => grpc.start_watch(from_seq).await,
            Backend::Http(_) => Err(ClientError::Unsupported(
                "watch is not available over plain HTTP".into(),
            )),
        }
    }

    pub(crate) async fn next_watch_event(
        &mut self,
    ) -> ClientResult<Option<storkv_proto::WireChangeEvent>> {
        match self {
            Backend::Native(native) => native.next_event().await,
            Backend::Ws(ws) => ws.next_event().await,
            Backend::Grpc(grpc) => grpc.next_event().await,
            Backend::Http(_) => Err(ClientError::Unsupported(
                "watch is not available on this transport".into(),
            )),
        }
    }

    pub(crate) async fn start_replication(
        &mut self,
        request_id: u64,
        from_seq: u64,
    ) -> ClientResult<()> {
        match self {
            Backend::Native(_) => {
                let response = self
                    .send(request_id, Request::ReplicationStart { from_seq })
                    .await?;
                if response == Response::Empty {
                    Ok(())
                } else {
                    Err(ClientError::Protocol(format!(
                        "Unexpected response: {response:?}"
                    )))
                }
            }
            _ => Err(ClientError::Unsupported(
                "replication needs the native transport".into(),
            )),
        }
    }

    pub(crate) async fn next_replication(&mut self) -> ClientResult<Option<Response>> {
        match self {
            Backend::Native(native) => native.next_replication().await,
            _ => Err(ClientError::Unsupported(
                "replication needs the native transport".into(),
            )),
        }
    }
}

pub(crate) fn decode_response(read_buf: &mut bytes::BytesMut) -> ClientResult<Option<Response>> {
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
