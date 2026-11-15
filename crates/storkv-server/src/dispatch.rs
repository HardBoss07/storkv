use std::path::Path;
use std::time::Instant;

use storkv_proto::{Request, Response};

use crate::server::Server;

mod conversions;
mod data;
mod scopes;
mod telemetry;
mod transactions;

pub use conversions::core_error_to_wire;

use conversions::result_to_empty;

impl Server {
    pub(crate) fn dispatch_request(&self, request: Request, _is_admin: bool) -> Response {
        let is_write = is_write_request(&request);
        let started = Instant::now();
        let response = self.route_request(request);
        if is_write {
            let elapsed_micros = started.elapsed().as_micros() as u64;
            self.stats.record_latency(elapsed_micros);
        }
        response
    }

    fn route_request(&self, request: Request) -> Response {
        match request {
            Request::Ping => Response::Pong,
            Request::HealthCheck => Response::HealthOk,
            Request::TelemetryQuery => self.handle_telemetry_query(),
            Request::Compact => result_to_empty(self.db.compact()),
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
            } => self.handle_scan(scope_id, start_key, end_key),
            Request::ScanPrefix { scope_id, prefix } => self.handle_scan_prefix(scope_id, prefix),
            Request::TxBegin => self.handle_tx_begin(),
            Request::TxCommit {
                read_seq,
                read_set,
                write_set,
            } => self.handle_tx_commit(read_seq, read_set, write_set),
            Request::AdminStats => Response::AdminStats(self.stats.snapshot(&self.db)),
            Request::WatchSubscribe => Response::Empty,
            other => Response::Error(storkv_proto::StorKvWireError::Protocol(format!(
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

fn is_write_request(request: &Request) -> bool {
    matches!(
        request,
        Request::Put { .. }
            | Request::PutWithTtl { .. }
            | Request::Delete { .. }
            | Request::DeleteRange { .. }
            | Request::Merge { .. }
            | Request::TxCommit { .. }
            | Request::DropScope { .. }
    )
}
