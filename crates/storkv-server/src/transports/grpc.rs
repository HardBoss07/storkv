//! gRPC listener and shared service plumbing.
//!
//! The generated `storkv.v1` types live in `generated`. Request conversion
//! helpers are in `conversions`, while the data and admin RPC impls live in
//! `service` and `admin`.

use std::sync::Arc;
use std::time::Duration;

use storkv_proto::{Request, Response, StorkvWireError};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request as GrpcRequest, Response as GrpcResponse, Status};

use crate::server::Server;
use crate::transports::{ConnectionGuard, TransportKind, TransportOptions};

mod admin;
mod conversions;
mod generated;
mod service;

use generated::storkv_admin_server::StorkvAdminServer;
use generated::storkv_server::StorkvServer;
pub use generated::{
    BackupEntry, ChangeEvent, CommitReply, CommitRequest, CreateScopeRequest, DeleteRangeRequest,
    DeleteRequest, Empty, GetReply, GetRequest, ListBackupsReply, MergeOp, MergeRequest, Mutation,
    Pair, PingReply, PutRequest, ReadEntry, ScanReply, ScanRequest,
    ScopeOptions as ProtoScopeOptions, ScopeRef, TelemetryReply, VerifyBackupReply,
    VerifyBackupRequest, WatchRequest,
};

const WATCH_CHANNEL_CAPACITY: usize = 128;
const WATCH_POLL_INTERVAL: Duration = Duration::from_millis(250);

const MUTATION_DELETE: u32 = 1;
const MUTATION_MERGE: u32 = 2;
const MUTATION_DELETE_RANGE: u32 = 3;

#[derive(Clone)]
/// Shared gRPC service state for the data and admin RPC services.
pub struct GrpcService {
    server: Arc<Server>,
    options: TransportOptions,
}

/// Serve the gRPC services on an already-bound TCP listener.
///
/// # Errors
///
/// Returns a transport error when the server cannot start or stops abnormally.
pub async fn serve(
    server: Arc<Server>,
    listener: TcpListener,
    options: TransportOptions,
) -> Result<(), tonic::transport::Error> {
    let _guard = ConnectionGuard::new(server.clone(), TransportKind::Grpc);
    let service = GrpcService {
        server: server.clone(),
        options,
    };
    let incoming = TcpListenerStream::new(listener);
    let mut shutdown_receiver = server.shutdown_sender.subscribe();
    tonic::transport::Server::builder()
        .add_service(StorkvServer::new(service.clone()))
        .add_service(StorkvAdminServer::new(service))
        .serve_with_incoming_shutdown(incoming, async move {
            let _ = shutdown_receiver.recv().await;
        })
        .await
}

impl GrpcService {
    fn is_admin<T>(&self, request: &GrpcRequest<T>) -> bool {
        if self.options.admin {
            return true;
        }
        let metadata = request.metadata();
        let header_token = metadata
            .get("x-storkv-admin-token")
            .and_then(|value| value.to_str().ok());
        let bearer_token = metadata
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        self.options.grants_admin(header_token.or(bearer_token))
    }

    fn dispatch<T>(&self, request: GrpcRequest<T>, payload: Request) -> Result<Response, Status> {
        let is_admin = self.is_admin(&request);
        self.server
            .stats
            .record_transport_request(TransportKind::Grpc.as_u8());
        let consistency = crate::server::ConnectionConsistency::new(self.server.consistency());
        match self
            .server
            .dispatch_request(payload, is_admin, &consistency)
        {
            Response::Error(error) => Err(status_from_wire(error)),
            other => Ok(other),
        }
    }

    fn ok<T>(
        &self,
        request: GrpcRequest<T>,
        payload: Request,
    ) -> Result<GrpcResponse<Empty>, Status> {
        self.dispatch(request, payload)
            .map(|_| GrpcResponse::new(Empty {}))
    }
}

fn status_from_wire(error: StorkvWireError) -> Status {
    let message = error.to_string();
    match error {
        StorkvWireError::Unauthorized => Status::permission_denied(message),
        StorkvWireError::ScopeNotFound(_)
        | StorkvWireError::ScopeIdNotFound(_)
        | StorkvWireError::SnapshotNotFound(_) => Status::not_found(message),
        StorkvWireError::WriteConflict { .. } => Status::aborted(message),
        StorkvWireError::WatchCursorExpired { .. } => Status::out_of_range(message),
        StorkvWireError::NotPrimary => Status::failed_precondition(message),
        StorkvWireError::StaleReplica { .. } => Status::unavailable(message),
        StorkvWireError::ScopeAlreadyExists(_) => Status::already_exists(message),
        StorkvWireError::TooManySubscribers { .. }
        | StorkvWireError::ReplayBudgetExceeded { .. } => Status::resource_exhausted(message),
        StorkvWireError::KeyTooLarge { .. }
        | StorkvWireError::ValueTooLarge { .. }
        | StorkvWireError::InvalidOperation(_)
        | StorkvWireError::Protocol(_) => Status::invalid_argument(message),
        StorkvWireError::Io(_) | StorkvWireError::Corruption(_) => Status::internal(message),
        _ => Status::unknown(message),
    }
}

fn not_found() -> Status {
    Status::internal("unexpected response type")
}
