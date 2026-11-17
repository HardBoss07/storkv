//! `StorKv` data-plane gRPC implementation.

use std::time::Duration;

use storkv_proto::{Request, Response, WireMergeOp};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request as GrpcRequest, Response as GrpcResponse, Status};

use super::conversions::{
    convert_event, fallback_scope, mutation, optional_bytes, scope_options, telemetry_reply,
};
use super::generated::stor_kv_server::StorKv;
use super::{
    ChangeEvent, CommitReply, CommitRequest, CreateScopeRequest, DeleteRangeRequest, DeleteRequest,
    GrpcService, MergeRequest, Pair, PingReply, PutRequest, ScanReply, ScanRequest, ScopeRef,
    TelemetryReply, WATCH_CHANNEL_CAPACITY, WATCH_POLL_INTERVAL, WatchRequest, not_found,
};

#[tonic::async_trait]
impl StorKv for GrpcService {
    async fn ping(
        &self,
        request: GrpcRequest<super::Empty>,
    ) -> Result<GrpcResponse<PingReply>, Status> {
        let response = self.dispatch(request, Request::Ping)?;
        let ok = matches!(response, Response::Pong);
        Ok(GrpcResponse::new(PingReply { ok }))
    }

    async fn health(
        &self,
        request: GrpcRequest<super::Empty>,
    ) -> Result<GrpcResponse<PingReply>, Status> {
        let response = self.dispatch(request, Request::HealthCheck)?;
        let ok = matches!(response, Response::HealthOk);
        Ok(GrpcResponse::new(PingReply { ok }))
    }

    async fn get(
        &self,
        request: GrpcRequest<super::GetRequest>,
    ) -> Result<GrpcResponse<super::GetReply>, Status> {
        let payload = {
            let message = request.get_ref();
            Request::Get {
                scope_id: message.scope_id,
                key: bytes::Bytes::from(message.key.clone()),
            }
        };
        match self.dispatch(request, payload)? {
            Response::Value(value) => Ok(GrpcResponse::new(match value {
                Some(value) => super::GetReply {
                    found: true,
                    value: value.to_vec(),
                },
                None => super::GetReply {
                    found: false,
                    value: Vec::new(),
                },
            })),
            _ => Err(not_found()),
        }
    }

    async fn put(
        &self,
        request: GrpcRequest<PutRequest>,
    ) -> Result<GrpcResponse<super::Empty>, Status> {
        let payload = {
            let message = request.get_ref();
            let key = bytes::Bytes::from(message.key.clone());
            let value = bytes::Bytes::from(message.value.clone());
            if message.ttl_ms == 0 {
                Request::Put {
                    scope_id: message.scope_id,
                    key,
                    val: value,
                }
            } else {
                Request::PutWithTtl {
                    scope_id: message.scope_id,
                    key,
                    val: value,
                    ttl: Duration::from_millis(message.ttl_ms),
                }
            }
        };
        self.ok(request, payload)
    }

    async fn delete(
        &self,
        request: GrpcRequest<DeleteRequest>,
    ) -> Result<GrpcResponse<super::Empty>, Status> {
        let payload = {
            let message = request.get_ref();
            Request::Delete {
                scope_id: message.scope_id,
                key: bytes::Bytes::from(message.key.clone()),
            }
        };
        self.ok(request, payload)
    }

    async fn delete_range(
        &self,
        request: GrpcRequest<DeleteRangeRequest>,
    ) -> Result<GrpcResponse<super::Empty>, Status> {
        let payload = {
            let message = request.get_ref();
            Request::DeleteRange {
                scope_id: message.scope_id,
                start_key: bytes::Bytes::from(message.start_key.clone()),
                end_key: bytes::Bytes::from(message.end_key.clone()),
            }
        };
        self.ok(request, payload)
    }

    async fn merge(
        &self,
        request: GrpcRequest<MergeRequest>,
    ) -> Result<GrpcResponse<super::Empty>, Status> {
        let payload = {
            let message = request.get_ref();
            let op = if message.custom.is_empty() {
                WireMergeOp::CounterAdd(message.counter_delta)
            } else {
                WireMergeOp::Custom(bytes::Bytes::from(message.custom.clone()))
            };
            Request::Merge {
                scope_id: message.scope_id,
                key: bytes::Bytes::from(message.key.clone()),
                op,
            }
        };
        self.ok(request, payload)
    }

    async fn scan(
        &self,
        request: GrpcRequest<ScanRequest>,
    ) -> Result<GrpcResponse<ScanReply>, Status> {
        let payload = {
            let message = request.get_ref();
            if message.prefix.is_empty() {
                Request::Scan {
                    scope_id: message.scope_id,
                    start_key: optional_bytes(&message.start_key),
                    end_key: optional_bytes(&message.end_key),
                }
            } else {
                Request::ScanPrefix {
                    scope_id: message.scope_id,
                    prefix: bytes::Bytes::from(message.prefix.clone()),
                }
            }
        };
        match self.dispatch(request, payload)? {
            Response::Scan(entries) => Ok(GrpcResponse::new(ScanReply {
                pairs: entries
                    .into_iter()
                    .map(|(key, value)| Pair {
                        key: key.to_vec(),
                        value: value.to_vec(),
                    })
                    .collect(),
            })),
            _ => Err(not_found()),
        }
    }

    async fn begin_tx(
        &self,
        request: GrpcRequest<super::Empty>,
    ) -> Result<GrpcResponse<super::generated::BeginTxReply>, Status> {
        match self.dispatch(request, Request::TxBegin)? {
            Response::TxStarted { read_seq } => {
                Ok(GrpcResponse::new(super::generated::BeginTxReply {
                    read_seq,
                }))
            }
            _ => Err(not_found()),
        }
    }

    async fn commit(
        &self,
        request: GrpcRequest<CommitRequest>,
    ) -> Result<GrpcResponse<CommitReply>, Status> {
        let payload = {
            let message = request.get_ref();
            let fallback = fallback_scope(&message.write_set);
            let read_set = message
                .read_set
                .iter()
                .map(|entry| {
                    let scope_id = if entry.scope_id == 0 {
                        fallback
                    } else {
                        entry.scope_id
                    };
                    (scope_id, bytes::Bytes::from(entry.key.clone()))
                })
                .collect();
            let write_set = message
                .write_set
                .iter()
                .filter_map(|entry| {
                    mutation(entry.clone()).map(|mutation| {
                        (
                            (entry.scope_id, bytes::Bytes::from(entry.key.clone())),
                            mutation,
                        )
                    })
                })
                .collect();
            Request::TxCommit {
                read_seq: message.read_seq,
                read_set,
                write_set,
            }
        };
        match self.dispatch(request, payload)? {
            Response::Empty => Ok(GrpcResponse::new(CommitReply { committed: true })),
            _ => Ok(GrpcResponse::new(CommitReply { committed: false })),
        }
    }

    async fn create_scope(
        &self,
        request: GrpcRequest<CreateScopeRequest>,
    ) -> Result<GrpcResponse<ScopeRef>, Status> {
        let payload = {
            let message = request.get_ref();
            Request::CreateScope {
                name: message.name.clone(),
                options: scope_options(message.options),
            }
        };
        match self.dispatch(request, payload)? {
            Response::Scope(handle) => Ok(GrpcResponse::new(ScopeRef {
                scope_id: handle.scope_id,
                name: handle.name,
            })),
            _ => Err(not_found()),
        }
    }

    async fn drop_scope(
        &self,
        request: GrpcRequest<ScopeRef>,
    ) -> Result<GrpcResponse<super::Empty>, Status> {
        let payload = {
            let message = request.get_ref();
            Request::DropScope {
                name: message.name.clone(),
            }
        };
        self.ok(request, payload)
    }

    type WatchStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<ChangeEvent, Status>> + Send>>;

    async fn watch(
        &self,
        request: GrpcRequest<WatchRequest>,
    ) -> Result<GrpcResponse<Self::WatchStream>, Status> {
        let (scope_id, all_scopes) = {
            let message = request.get_ref();
            (message.scope_id, message.all_scopes)
        };
        let receiver = self.server.db().watch();
        let (sender, stream_receiver) =
            mpsc::channel::<Result<ChangeEvent, Status>>(WATCH_CHANNEL_CAPACITY);
        let _watch_task = tokio::task::spawn_blocking(move || {
            while !sender.is_closed() {
                match receiver.recv_timeout(WATCH_POLL_INTERVAL) {
                    Ok(event) => {
                        let Some(event) = convert_event(event) else {
                            continue;
                        };
                        if !all_scopes && event.scope_id != scope_id {
                            continue;
                        }
                        if sender.blocking_send(Ok(event)).is_err() {
                            break;
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        let stream = ReceiverStream::new(stream_receiver);
        Ok(GrpcResponse::new(Box::pin(stream)))
    }

    async fn telemetry(
        &self,
        request: GrpcRequest<super::Empty>,
    ) -> Result<GrpcResponse<TelemetryReply>, Status> {
        self.dispatch(request, Request::TelemetryQuery)?;
        Ok(GrpcResponse::new(telemetry_reply(&self.server)))
    }
}
