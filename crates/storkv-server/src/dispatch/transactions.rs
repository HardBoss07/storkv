//! Explicit transaction request handlers built on one engine transaction.

use bytes::Bytes;
use storkv_proto::{Response, StorkvWireError, WireTxMutation};

use super::conversions::core_error_to_wire;
use crate::server::Server;

impl Server {
    pub(super) fn handle_tx_begin(&self) -> Response {
        let transaction = self.db.begin_tx();
        Response::TxStarted {
            read_seq: transaction.read_seq(),
        }
    }

    pub(super) fn handle_tx_commit(
        &self,
        read_seq: u64,
        read_set: Vec<(u32, Bytes)>,
        write_set: Vec<((u32, Bytes), WireTxMutation)>,
    ) -> Response {
        let read_scope_ids: Vec<u32> = read_set.iter().map(|(scope_id, _)| *scope_id).collect();
        let write_scope_ids: Vec<u32> = write_set
            .iter()
            .map(|((scope_id, _), _)| *scope_id)
            .collect();
        let mut context = storkv_core::mvcc::TransactionContext::new(read_seq);
        for (scope_id, user_key) in read_set {
            context.record_read(scope_id, user_key);
        }
        for ((scope_id, user_key), mutation) in write_set {
            if let Err(error) = apply_tx_mutation(&mut context, scope_id, user_key, mutation) {
                return Response::Error(error);
            }
        }

        match self.db.inner().commit_tx(context) {
            Ok(()) => {
                for scope_id in read_scope_ids {
                    self.stats.record_read(scope_id);
                }
                for scope_id in write_scope_ids {
                    self.stats.record_write(scope_id);
                }
                Response::Empty
            }
            Err(error) => Response::Error(core_error_to_wire(error)),
        }
    }
}

fn apply_tx_mutation(
    context: &mut storkv_core::mvcc::TransactionContext,
    scope_id: u32,
    user_key: Bytes,
    mutation: WireTxMutation,
) -> Result<(), StorkvWireError> {
    match mutation {
        WireTxMutation::Put { val, expires_at_ms } => {
            context.put(scope_id, user_key, val, expires_at_ms);
            Ok(())
        }
        WireTxMutation::Delete => {
            context.delete(scope_id, user_key);
            Ok(())
        }
        WireTxMutation::Merge { operand } => {
            context.merge(scope_id, user_key, operand);
            Ok(())
        }
        WireTxMutation::DeleteRange { end_key } => {
            context.write_set.insert(
                (scope_id, user_key),
                storkv_core::mvcc::TxMutation::DeleteRange { end_key },
            );
            Ok(())
        }
        _ => Err(StorkvWireError::InvalidOperation(
            "unsupported transaction mutation".into(),
        )),
    }
}
