//! Request routing for [`crate::Server::dispatch_request`].
//!
//! The router applies admin-only access, replica write rejection, and
//! consistency checks before it calls one handler. The handlers live in the
//! child modules.

use std::path::Path;
use std::time::Instant;

use storkv_proto::{Request, Response};

use crate::metrics::OpKind;
use crate::server::{ConnectionConsistency, Server};

mod backup;
mod conversions;
mod data;
mod scopes;
mod telemetry;
mod transactions;

pub use conversions::core_error_to_wire;

use conversions::result_to_empty;

impl Server {
    pub(crate) fn dispatch_request(
        &self,
        request: Request,
        is_admin: bool,
        consistency: &ConnectionConsistency,
    ) -> Response {
        if !is_admin && request.is_admin_only() && self.strict_admin() {
            return Response::Error(storkv_proto::StorkvWireError::Unauthorized);
        }
        if let Some(error) = self.replica_rejection(&request, consistency) {
            return Response::Error(error);
        }
        let is_write = request.is_write();
        let operation = OpKind::from_request(&request);
        let started = Instant::now();
        let response = self.route_request(request, consistency);
        let elapsed_micros = started.elapsed().as_micros() as u64;
        if let Some(kind) = operation {
            self.stats.record_operation(kind, elapsed_micros);
        }
        if is_write {
            self.stats.record_latency(elapsed_micros);
        }
        response
    }

    fn replica_rejection(
        &self,
        request: &Request,
        consistency: &ConnectionConsistency,
    ) -> Option<storkv_proto::StorkvWireError> {
        if self.role() != crate::ReplicationRole::Replica {
            return None;
        }
        if request.is_write() {
            return Some(storkv_proto::StorkvWireError::NotPrimary);
        }
        if !is_data_read(request) {
            return None;
        }
        match consistency.get() {
            storkv_proto::Consistency::Strong => Some(storkv_proto::StorkvWireError::NotPrimary),
            storkv_proto::Consistency::BoundedStaleness { max_lag_ms } => {
                let lag_ms = self.replication_lag_ms();
                (lag_ms > max_lag_ms)
                    .then_some(storkv_proto::StorkvWireError::StaleReplica { lag_ms, max_lag_ms })
            }
            storkv_proto::Consistency::Eventual => None,
            _ => None,
        }
    }

    fn route_request(&self, request: Request, consistency: &ConnectionConsistency) -> Response {
        match request {
            Request::Ping => Response::Pong,
            Request::HealthCheck => Response::HealthOk,
            Request::TelemetryQuery => self.handle_telemetry_query(),
            Request::Compact => result_to_empty(self.db.compact()),
            Request::CompactRange { scope, start, end } => result_to_empty(self.db.compact_range(
                &scope,
                start.as_deref(),
                end.as_deref(),
            )),
            Request::CreateScope { name, options } => self.handle_create_scope(&name, options),
            Request::DropScope { name } => self.handle_drop_scope(&name),
            Request::CreateSnapshot { name } => result_to_empty(self.db.create_snapshot(name)),
            Request::UnpinSnapshot { name } => result_to_empty(self.db.unpin_snapshot(&name)),
            Request::Checkpoint { dest_dir } => {
                result_to_empty(self.db.checkpoint(Path::new(&dest_dir)))
            }
            Request::Get { scope_id, key } => self.handle_get(scope_id, key),
            Request::Put { scope_id, key, val } => self.handle_put(scope_id, key, val),
            Request::PutWithTtl {
                scope_id,
                key,
                val,
                ttl,
            } => self.handle_put_with_ttl(scope_id, key, val, ttl),
            Request::PutIf {
                scope_id,
                key,
                condition,
                val,
                ttl,
            } => self.handle_put_if(scope_id, key, condition, val, ttl),
            Request::Delete { scope_id, key } => self.handle_delete(scope_id, key),
            Request::DeleteRange {
                scope_id,
                start_key,
                end_key,
            } => self.handle_delete_range(scope_id, start_key, end_key),
            Request::Merge { scope_id, key, op } => self.handle_merge(scope_id, key, op),
            Request::Scan {
                scope_id,
                start_key,
                end_key,
                limit,
                reverse,
                keys_only,
            } => self.handle_scan(scope_id, start_key, end_key, limit, reverse, keys_only),
            Request::ScanPrefix {
                scope_id,
                prefix,
                limit,
                reverse,
                keys_only,
            } => self.handle_scan_prefix(scope_id, prefix, limit, reverse, keys_only),
            Request::TxBegin => self.handle_tx_begin(),
            Request::TxCommit {
                read_seq,
                read_set,
                write_set,
            } => self.handle_tx_commit(read_seq, read_set, write_set),
            Request::AdminStats => Response::AdminStats(self.stats.snapshot(&self.db)),
            Request::TriggerBackup { wait } => self.handle_trigger_backup(wait),
            Request::BackupStatus => self.handle_backup_status(),
            Request::VerifyBackup { target_seq } => self.handle_verify_backup(target_seq),
            Request::ListBackups => self.handle_list_backups(),
            Request::KeyMeta { scope_id, key } => self.handle_key_meta(scope_id, key),
            Request::WatchSubscribe | Request::WatchFrom { .. } => Response::Empty,
            Request::ReplicationStart { .. } => Response::Empty,
            Request::ConfigureConsistency { consistency: level } => {
                consistency.set(level);
                Response::Empty
            }
            other => Response::Error(storkv_proto::StorkvWireError::Protocol(format!(
                "Unsupported request: {other:?}"
            ))),
        }
    }

    fn complete_write(&self, scope_id: u32, result: storkv_core::Result<()>) -> Response {
        match result {
            Ok(()) => {
                self.stats.record_write(scope_id);
                Response::Empty
            }
            Err(error) => Response::Error(core_error_to_wire(error)),
        }
    }
}

fn is_data_read(request: &Request) -> bool {
    matches!(
        request,
        Request::Get { .. }
            | Request::Scan { .. }
            | Request::ScanPrefix { .. }
            | Request::KeyMeta { .. }
    )
}
