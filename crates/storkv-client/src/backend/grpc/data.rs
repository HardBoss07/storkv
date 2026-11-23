//! Data-plane request handling for the gRPC backend.

use bytes::Bytes;
use storkv_proto::{Request, Response, WireMergeOp, WireScopeHandle};

use super::GrpcBackend;
use super::{mutation_to_proto, pairs, scope_options, telemetry};
use crate::grpc::{
    self, DeleteRangeRequest, DeleteRequest, Empty, GetRequest, MergeOp, MergeRequest, PutRequest,
    ReadEntry, ScanRequest, ScopeRef, merge_op,
};
use crate::transport::{ClientError, ClientResult};

impl GrpcBackend {
    pub(super) async fn handle_data(&mut self, request: Request) -> ClientResult<Response> {
        let response = match request {
            Request::Ping => Response::Pong,
            Request::HealthCheck => Response::HealthOk,
            Request::TelemetryQuery => Response::Telemetry(telemetry(
                self.client.telemetry(Empty {}).await?.into_inner(),
            )),
            Request::Get { scope_id, key } => {
                let reply = self
                    .client
                    .get(GetRequest {
                        scope_id,
                        key: key.to_vec(),
                    })
                    .await?
                    .into_inner();
                Response::Value(reply.found.then(|| Bytes::from(reply.value)))
            }
            Request::Put { scope_id, key, val } => {
                self.put(scope_id, key, val, 0).await?;
                Response::Empty
            }
            Request::PutWithTtl {
                scope_id,
                key,
                val,
                ttl,
            } => {
                self.put(scope_id, key, val, ttl.as_millis() as u64).await?;
                Response::Empty
            }
            Request::Delete { scope_id, key } => {
                self.client
                    .delete(DeleteRequest {
                        scope_id,
                        key: key.to_vec(),
                    })
                    .await?;
                Response::Empty
            }
            Request::DeleteRange {
                scope_id,
                start_key,
                end_key,
            } => {
                self.client
                    .delete_range(DeleteRangeRequest {
                        scope_id,
                        start_key: start_key.to_vec(),
                        end_key: end_key.to_vec(),
                    })
                    .await?;
                Response::Empty
            }
            Request::Merge { scope_id, key, op } => {
                self.client
                    .merge(MergeRequest {
                        scope_id,
                        key: key.to_vec(),
                        op: Some(merge_op_to_proto(op)),
                        ..MergeRequest::default()
                    })
                    .await?;
                Response::Empty
            }
            Request::Scan {
                scope_id,
                start_key,
                end_key,
            } => {
                let reply = self
                    .client
                    .scan(ScanRequest {
                        scope_id,
                        start_key: start_key.map(|key| key.to_vec()).unwrap_or_default(),
                        end_key: end_key.map(|key| key.to_vec()).unwrap_or_default(),
                        prefix: Vec::new(),
                    })
                    .await?
                    .into_inner();
                Response::Scan(pairs(reply.pairs))
            }
            Request::ScanPrefix { scope_id, prefix } => {
                let reply = self
                    .client
                    .scan(ScanRequest {
                        scope_id,
                        start_key: Vec::new(),
                        end_key: Vec::new(),
                        prefix: prefix.to_vec(),
                    })
                    .await?
                    .into_inner();
                Response::Scan(pairs(reply.pairs))
            }
            Request::TxBegin => {
                let reply = self.client.begin_tx(Empty {}).await?.into_inner();
                Response::TxStarted {
                    read_seq: reply.read_seq,
                }
            }
            Request::TxCommit {
                read_seq,
                read_set,
                write_set,
            } => {
                let read_set = read_set
                    .into_iter()
                    .map(|(scope_id, key)| ReadEntry {
                        scope_id,
                        key: key.to_vec(),
                    })
                    .collect();
                let write_set = write_set
                    .into_iter()
                    .map(|((scope_id, key), mutation)| mutation_to_proto(scope_id, key, mutation))
                    .collect();
                let reply = self
                    .client
                    .commit(grpc::CommitRequest {
                        read_seq,
                        read_set,
                        write_set,
                    })
                    .await?
                    .into_inner();
                if reply.committed {
                    Response::Empty
                } else {
                    Response::Error(storkv_proto::StorkvWireError::InvalidOperation(
                        "transaction was not committed".into(),
                    ))
                }
            }
            Request::CreateScope { name, options } => {
                let reply = self
                    .client
                    .create_scope(grpc::CreateScopeRequest {
                        name,
                        options: Some(scope_options(options)),
                    })
                    .await?
                    .into_inner();
                Response::Scope(WireScopeHandle {
                    scope_id: reply.scope_id,
                    name: reply.name,
                })
            }
            Request::DropScope { name } => {
                self.client
                    .drop_scope(ScopeRef { scope_id: 0, name })
                    .await?;
                Response::Empty
            }
            _ => {
                return Err(ClientError::Unsupported(
                    "request is not supported over gRPC".into(),
                ));
            }
        };
        Ok(response)
    }

    async fn put(
        &mut self,
        scope_id: u32,
        key: Bytes,
        value: Bytes,
        ttl_ms: u64,
    ) -> ClientResult<()> {
        self.client
            .put(PutRequest {
                scope_id,
                key: key.to_vec(),
                value: value.to_vec(),
                ttl_ms,
            })
            .await?;
        Ok(())
    }
}

fn merge_op_to_proto(op: WireMergeOp) -> MergeOp {
    let mut message = MergeOp::default();
    match op {
        WireMergeOp::CounterAdd(delta) => {
            message.kind = merge_op::Kind::CounterAdd.into();
            message.i64_value = delta;
        }
        WireMergeOp::Custom(bytes) => {
            message.kind = merge_op::Kind::Custom.into();
            message.data = bytes.to_vec();
        }
        WireMergeOp::AppendBytes { data, max_len } => {
            message.kind = merge_op::Kind::AppendBytes.into();
            message.data = data.to_vec();
            message.max_len = max_len;
        }
        WireMergeOp::MaxI64(value) => {
            message.kind = merge_op::Kind::MaxI64.into();
            message.i64_value = value;
        }
        WireMergeOp::MinI64(value) => {
            message.kind = merge_op::Kind::MinI64.into();
            message.i64_value = value;
        }
        WireMergeOp::BitOr(bits) => {
            message.kind = merge_op::Kind::BitOr.into();
            message.u64_value = bits;
        }
        WireMergeOp::BitAnd(bits) => {
            message.kind = merge_op::Kind::BitAnd.into();
            message.u64_value = bits;
        }
        _ => message.kind = merge_op::Kind::Unspecified.into(),
    }
    message
}
