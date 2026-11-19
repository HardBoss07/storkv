//! `Storkv` data-plane gRPC implementation.

use storkv_proto::{Request, Response};
use tonic::{Request as GrpcRequest, Response as GrpcResponse, Status};

use super::super::conversions::telemetry_reply;
use super::super::generated::storkv_server::Storkv;
use super::super::{
    CommitReply, CommitRequest, CreateScopeRequest, DeleteRangeRequest, DeleteRequest, GrpcService,
    MergeRequest, Pair, PingReply, PutRequest, ScanReply, ScanRequest, ScopeRef, TelemetryReply,
    not_found,
};
use super::build;

#[tonic::async_trait]
impl Storkv for GrpcService {
    async fn ping(
        &self,
        request: GrpcRequest<super::super::Empty>,
    ) -> Result<GrpcResponse<PingReply>, Status> {
        let response = self.dispatch(request, Request::Ping)?;
        let ok = matches!(response, Response::Pong);
        Ok(GrpcResponse::new(PingReply { ok }))
    }

    async fn health(
        &self,
        request: GrpcRequest<super::super::Empty>,
    ) -> Result<GrpcResponse<PingReply>, Status> {
        let response = self.dispatch(request, Request::HealthCheck)?;
        let ok = matches!(response, Response::HealthOk);
        Ok(GrpcResponse::new(PingReply { ok }))
    }

    async fn get(
        &self,
        request: GrpcRequest<super::super::GetRequest>,
    ) -> Result<GrpcResponse<super::super::GetReply>, Status> {
        let payload = build::get(request.get_ref());
        match self.dispatch(request, payload)? {
            Response::Value(value) => Ok(GrpcResponse::new(match value {
                Some(value) => super::super::GetReply {
                    found: true,
                    value: value.to_vec(),
                },
                None => super::super::GetReply {
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
    ) -> Result<GrpcResponse<super::super::Empty>, Status> {
        let payload = build::put(request.get_ref());
        self.ok(request, payload)
    }

    async fn delete(
        &self,
        request: GrpcRequest<DeleteRequest>,
    ) -> Result<GrpcResponse<super::super::Empty>, Status> {
        let payload = build::delete(request.get_ref());
        self.ok(request, payload)
    }

    async fn delete_range(
        &self,
        request: GrpcRequest<DeleteRangeRequest>,
    ) -> Result<GrpcResponse<super::super::Empty>, Status> {
        let payload = build::delete_range(request.get_ref());
        self.ok(request, payload)
    }

    async fn merge(
        &self,
        request: GrpcRequest<MergeRequest>,
    ) -> Result<GrpcResponse<super::super::Empty>, Status> {
        let payload = build::merge(request.get_ref());
        self.ok(request, payload)
    }

    async fn scan(
        &self,
        request: GrpcRequest<ScanRequest>,
    ) -> Result<GrpcResponse<ScanReply>, Status> {
        let payload = build::scan(request.get_ref());
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
        request: GrpcRequest<super::super::Empty>,
    ) -> Result<GrpcResponse<super::super::generated::BeginTxReply>, Status> {
        match self.dispatch(request, Request::TxBegin)? {
            Response::TxStarted { read_seq } => {
                Ok(GrpcResponse::new(super::super::generated::BeginTxReply {
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
        let payload = build::commit(request.get_ref());
        match self.dispatch(request, payload)? {
            Response::Empty => Ok(GrpcResponse::new(CommitReply { committed: true })),
            _ => Ok(GrpcResponse::new(CommitReply { committed: false })),
        }
    }

    async fn create_scope(
        &self,
        request: GrpcRequest<CreateScopeRequest>,
    ) -> Result<GrpcResponse<ScopeRef>, Status> {
        let payload = build::create_scope(request.get_ref());
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
    ) -> Result<GrpcResponse<super::super::Empty>, Status> {
        let payload = build::drop_scope(request.get_ref());
        self.ok(request, payload)
    }

    async fn telemetry(
        &self,
        request: GrpcRequest<super::super::Empty>,
    ) -> Result<GrpcResponse<TelemetryReply>, Status> {
        self.dispatch(request, Request::TelemetryQuery)?;
        Ok(GrpcResponse::new(telemetry_reply(&self.server)))
    }

    type WatchStream = super::watch::WatchStream;

    async fn watch(
        &self,
        request: GrpcRequest<super::super::WatchRequest>,
    ) -> Result<GrpcResponse<Self::WatchStream>, Status> {
        super::watch::watch_stream(self, request)
    }
}
