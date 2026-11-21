use crate::transport::{ClientError, ClientResult};

/// Connection transport selected from a `storkv://`, `unix://`, `http://`,
/// `ws://`, or `grpc://` URI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// Native protocol over TCP.
    NativeTcp,
    /// Native protocol over a Unix-domain socket.
    NativeUnix,
    /// HTTP/JSON transport.
    Http,
    /// WebSocket transport.
    WebSocket,
    /// gRPC transport.
    Grpc,
}

impl TransportKind {
    /// Split a URI into its transport kind and the backend-specific target.
    ///
    /// Bare `host:port` addresses and filesystem-like paths keep their
    /// historical meaning: native TCP and native Unix-domain sockets.
    pub fn parse_uri(uri: &str) -> ClientResult<(Self, String)> {
        let trimmed = uri.trim();
        if trimmed.is_empty() {
            return Err(ClientError::Protocol("empty connection URI".into()));
        }
        if let Some(rest) = trimmed.strip_prefix("storkv://") {
            return Ok((Self::NativeTcp, require_target(rest, uri)?));
        }
        if let Some(rest) = trimmed.strip_prefix("tcp://") {
            return Ok((Self::NativeTcp, require_target(rest, uri)?));
        }
        if let Some(rest) = trimmed.strip_prefix("unix://") {
            let path = rest.trim_start_matches('/');
            let absolute = format!("/{path}");
            return Ok((Self::NativeUnix, absolute));
        }
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return Ok((Self::Http, trimmed.to_string()));
        }
        if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
            return Ok((Self::WebSocket, trimmed.to_string()));
        }
        if let Some(rest) = trimmed.strip_prefix("grpc://") {
            return Ok((Self::Grpc, require_target(rest, uri)?));
        }
        if trimmed.contains("://") {
            let scheme = trimmed.split("://").next().unwrap_or_default();
            return Err(ClientError::Protocol(format!(
                "unsupported transport scheme: {scheme}"
            )));
        }
        if storkv_proto::is_unix_socket_addr(trimmed) {
            let path = storkv_proto::unix_socket_path(trimmed);
            if path.is_empty() {
                return Err(ClientError::Protocol(format!(
                    "connection URI is missing a target: {uri}"
                )));
            }
            return Ok((Self::NativeUnix, path.to_string()));
        }
        Ok((Self::NativeTcp, trimmed.to_string()))
    }
}

fn require_target(target: &str, uri: &str) -> ClientResult<String> {
    if target.is_empty() {
        return Err(ClientError::Protocol(format!(
            "connection URI is missing a target: {uri}"
        )));
    }
    Ok(target.trim_end_matches('/').to_string())
}
