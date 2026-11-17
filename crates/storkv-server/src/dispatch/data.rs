use bytes::Bytes;
use storkv_core::{MergeOp, ScopeHandle};
use storkv_proto::{Response, StorKvWireError, WireMergeOp};

use super::conversions::core_error_to_wire;
use crate::server::Server;

impl Server {
    pub(super) fn handle_get(&self, scope_id: u32, user_key: Bytes) -> Response {
        match self.db.get(&scope_handle(scope_id), user_key) {
            Ok(value) => {
                self.stats.record_read(scope_id);
                Response::Value(value)
            }
            Err(error) => Response::Error(core_error_to_wire(error)),
        }
    }

    pub(super) fn handle_put(
        &self,
        scope_id: u32,
        user_key: Bytes,
        entry_value: Bytes,
    ) -> Response {
        self.complete_write(
            scope_id,
            self.db.put(&scope_handle(scope_id), user_key, entry_value),
        )
    }

    pub(super) fn handle_put_with_ttl(
        &self,
        scope_id: u32,
        user_key: Bytes,
        entry_value: Bytes,
        ttl: std::time::Duration,
    ) -> Response {
        self.complete_write(
            scope_id,
            self.db
                .put_with_ttl(&scope_handle(scope_id), user_key, entry_value, ttl),
        )
    }

    pub(super) fn handle_delete(&self, scope_id: u32, user_key: Bytes) -> Response {
        self.complete_write(scope_id, self.db.delete(&scope_handle(scope_id), user_key))
    }

    pub(super) fn handle_delete_range(
        &self,
        scope_id: u32,
        start_key: Bytes,
        end_key: Bytes,
    ) -> Response {
        self.complete_write(
            scope_id,
            self.db
                .delete_range(&scope_handle(scope_id), start_key, end_key),
        )
    }

    pub(super) fn handle_merge(&self, scope_id: u32, user_key: Bytes, op: WireMergeOp) -> Response {
        let merge_op = match op {
            WireMergeOp::CounterAdd(delta) => MergeOp::CounterAdd(delta),
            WireMergeOp::Custom(operand) => MergeOp::Custom(operand),
            _ => {
                return Response::Error(StorKvWireError::InvalidOperation(
                    "unsupported merge operator".into(),
                ));
            }
        };
        self.complete_write(
            scope_id,
            self.db.merge(&scope_handle(scope_id), user_key, merge_op),
        )
    }

    pub(super) fn handle_scan(
        &self,
        scope_id: u32,
        start_key: Option<Bytes>,
        end_key: Option<Bytes>,
    ) -> Response {
        match self.db.scan(
            &scope_handle(scope_id),
            start_key.as_deref(),
            end_key.as_deref(),
        ) {
            Ok(entries) => {
                self.stats.record_read(scope_id);
                Response::Scan(entries)
            }
            Err(error) => Response::Error(core_error_to_wire(error)),
        }
    }

    pub(super) fn handle_key_meta(&self, scope_id: u32, user_key: Bytes) -> Response {
        match self.db.key_meta(&scope_handle(scope_id), user_key) {
            Ok(meta) => {
                self.stats.record_read(scope_id);
                Response::KeyMeta(meta.map(|meta| storkv_proto::WireKeyMeta {
                    expires_at_ms: meta.expires_at_ms,
                    seq: meta.seq,
                }))
            }
            Err(error) => Response::Error(core_error_to_wire(error)),
        }
    }

    pub(super) fn handle_scan_prefix(&self, scope_id: u32, prefix: Bytes) -> Response {
        match self.db.scan_prefix(&scope_handle(scope_id), prefix) {
            Ok(entries) => {
                self.stats.record_read(scope_id);
                Response::Scan(entries)
            }
            Err(error) => Response::Error(core_error_to_wire(error)),
        }
    }
}

fn scope_handle(scope_id: u32) -> ScopeHandle {
    ScopeHandle {
        scope_id,
        name: String::new(),
    }
}
