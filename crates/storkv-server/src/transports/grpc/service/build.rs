//! Request payload builders for the data-plane gRPC handlers.

use std::time::Duration;

use storkv_proto::{Request, WireMergeOp};

use crate::transports::grpc::generated::merge_op;

use super::super::conversions::{fallback_scope, mutation, optional_bytes, scope_options};
use super::super::{
    CommitRequest, CreateScopeRequest, DeleteRangeRequest, DeleteRequest, GetRequest, MergeOp,
    MergeRequest, PutRequest, ScanRequest, ScopeRef,
};

pub(super) fn get(message: &GetRequest) -> Request {
    Request::Get {
        scope_id: message.scope_id,
        key: bytes::Bytes::from(message.key.clone()),
    }
}

pub(super) fn put(message: &PutRequest) -> Request {
    let key = bytes::Bytes::from(message.key.clone());
    let val = bytes::Bytes::from(message.value.clone());
    if message.ttl_ms == 0 {
        Request::Put {
            scope_id: message.scope_id,
            key,
            val,
        }
    } else {
        Request::PutWithTtl {
            scope_id: message.scope_id,
            key,
            val,
            ttl: Duration::from_millis(message.ttl_ms),
        }
    }
}

pub(super) fn delete(message: &DeleteRequest) -> Request {
    Request::Delete {
        scope_id: message.scope_id,
        key: bytes::Bytes::from(message.key.clone()),
    }
}

pub(super) fn delete_range(message: &DeleteRangeRequest) -> Request {
    Request::DeleteRange {
        scope_id: message.scope_id,
        start_key: bytes::Bytes::from(message.start_key.clone()),
        end_key: bytes::Bytes::from(message.end_key.clone()),
    }
}

pub(super) fn merge(message: &MergeRequest) -> Request {
    let op = match message.op.as_ref() {
        Some(op) if op.kind() != merge_op::Kind::Unspecified => typed_merge_op(op),
        _ if message.custom.is_empty() => WireMergeOp::CounterAdd(message.counter_delta),
        _ => WireMergeOp::Custom(bytes::Bytes::from(message.custom.clone())),
    };
    Request::Merge {
        scope_id: message.scope_id,
        key: bytes::Bytes::from(message.key.clone()),
        op,
    }
}

fn typed_merge_op(op: &MergeOp) -> WireMergeOp {
    match op.kind() {
        merge_op::Kind::CounterAdd => WireMergeOp::CounterAdd(op.i64_value),
        merge_op::Kind::Custom => WireMergeOp::Custom(bytes::Bytes::from(op.data.clone())),
        merge_op::Kind::AppendBytes => WireMergeOp::AppendBytes {
            data: bytes::Bytes::from(op.data.clone()),
            max_len: op.max_len,
        },
        merge_op::Kind::MaxI64 => WireMergeOp::MaxI64(op.i64_value),
        merge_op::Kind::MinI64 => WireMergeOp::MinI64(op.i64_value),
        merge_op::Kind::BitOr => WireMergeOp::BitOr(op.u64_value),
        merge_op::Kind::BitAnd => WireMergeOp::BitAnd(op.u64_value),
        merge_op::Kind::Unspecified => WireMergeOp::CounterAdd(op.i64_value),
    }
}

pub(super) fn scan(message: &ScanRequest) -> Request {
    if message.prefix.is_empty() {
        Request::Scan {
            scope_id: message.scope_id,
            start_key: optional_bytes(&message.start_key),
            end_key: optional_bytes(&message.end_key),
            limit: None,
            reverse: false,
            keys_only: false,
        }
    } else {
        Request::ScanPrefix {
            scope_id: message.scope_id,
            prefix: bytes::Bytes::from(message.prefix.clone()),
            limit: None,
            reverse: false,
            keys_only: false,
        }
    }
}

pub(super) fn commit(message: &CommitRequest) -> Request {
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
}

pub(super) fn create_scope(message: &CreateScopeRequest) -> Request {
    Request::CreateScope {
        name: message.name.clone(),
        options: scope_options(message.options),
    }
}

pub(super) fn drop_scope(message: &ScopeRef) -> Request {
    Request::DropScope {
        name: message.name.clone(),
    }
}
