use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request as HttpRequest;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use storkv_proto::{Message, MessagePayload, Request, Response};

use crate::transport::{ClientError, ClientResult};

/// HTTP/1.1 + HTTP/2 backend using the binary `/v1/rpc` frame tunnel.
pub(crate) struct HttpBackend {
    client: Client<HttpConnector, Full<Bytes>>,
    endpoint: String,
}

impl HttpBackend {
    pub(crate) async fn connect(base: &str) -> ClientResult<Self> {
        let client = Client::builder(TokioExecutor::new()).build_http();
        Ok(Self {
            client,
            endpoint: rpc_endpoint(base),
        })
    }

    pub(crate) async fn send(
        &mut self,
        request_id: u64,
        request: Request,
    ) -> ClientResult<Response> {
        let encoded = Message::new_request(request_id, request).encode();
        let http_request = HttpRequest::builder()
            .method("POST")
            .uri(&self.endpoint)
            .header("content-type", "application/octet-stream")
            .body(Full::new(encoded))
            .map_err(|error| ClientError::Protocol(error.to_string()))?;
        let http_response = self
            .client
            .request(http_request)
            .await
            .map_err(|error| ClientError::Http(error.to_string()))?;
        let status = http_response.status();
        let body = http_response
            .into_body()
            .collect()
            .await
            .map_err(|error| ClientError::Http(error.to_string()))?
            .to_bytes();
        if !status.is_success() {
            return Err(ClientError::Http(format!(
                "HTTP {}: {}",
                status.as_u16(),
                String::from_utf8_lossy(&body)
            )));
        }
        let message = Message::decode(&mut bytes::BytesMut::from(body.as_ref()))?
            .ok_or_else(|| ClientError::Protocol("truncated HTTP response frame".into()))?;
        match message.payload {
            MessagePayload::Response(Response::Error(error)) => Err(ClientError::Server(error)),
            MessagePayload::Response(response) => Ok(response),
            MessagePayload::Request(_) => Err(ClientError::Protocol(
                "expected a response frame from /v1/rpc".into(),
            )),
        }
    }
}

fn rpc_endpoint(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/v1/rpc") {
        return trimmed.to_string();
    }
    format!("{trimmed}/v1/rpc")
}
