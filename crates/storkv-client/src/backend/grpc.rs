//! gRPC backend implementing the wire protocol over h2c.

use bytes::Bytes;
use storkv_proto::{
    Request, Response, WireChangeEvent, WireCompactionStyle, WireDurability, WireRetention,
    WireScopeOptions, WireTelemetrySnapshot, WireTxMutation,
};
use tonic::Streaming;
use tonic::transport::Channel;

use crate::grpc::{
    self, Mutation, Pair, storkv_admin_client::StorkvAdminClient, storkv_client::StorkvClient,
};
use crate::transport::{ClientError, ClientResult};

mod admin;
mod data;

/// gRPC backend using the generated `storkv.v1` clients over h2c.
pub(crate) struct GrpcBackend {
    client: StorkvClient<Channel>,
    admin: StorkvAdminClient<Channel>,
    watch: Option<Streaming<grpc::ChangeEvent>>,
}

impl GrpcBackend {
    pub(crate) async fn connect(target: &str) -> ClientResult<Self> {
        let endpoint = if target.starts_with("http://") || target.starts_with("https://") {
            target.to_string()
        } else {
            format!("http://{target}")
        };
        let channel = Channel::from_shared(endpoint)
            .map_err(|error| ClientError::Grpc(error.to_string()))?
            .connect()
            .await
            .map_err(|error| ClientError::Grpc(error.to_string()))?;
        Ok(Self {
            client: StorkvClient::new(channel.clone()),
            admin: StorkvAdminClient::new(channel),
            watch: None,
        })
    }

    pub(crate) async fn send(&mut self, request: Request) -> ClientResult<Response> {
        match request {
            Request::WatchSubscribe => {
                self.start_watch(None).await?;
                Ok(Response::Empty)
            }
            Request::WatchFrom { from_seq } => {
                self.start_watch(Some(from_seq)).await?;
                Ok(Response::Empty)
            }
            request @ (Request::CreateSnapshot { .. }
            | Request::UnpinSnapshot { .. }
            | Request::Checkpoint { .. }
            | Request::Compact
            | Request::AdminStats
            | Request::TriggerBackup { .. }
            | Request::BackupStatus
            | Request::VerifyBackup { .. }
            | Request::ListBackups) => self.handle_admin(request).await,
            request => self.handle_data(request).await,
        }
    }

    pub(crate) async fn start_watch(&mut self, from_seq: Option<u64>) -> ClientResult<()> {
        let stream = self
            .client
            .watch(grpc::WatchRequest {
                scope_id: 0,
                all_scopes: true,
                from_seq: from_seq.unwrap_or(0),
            })
            .await?
            .into_inner();
        self.watch = Some(stream);
        Ok(())
    }

    pub(crate) async fn next_event(&mut self) -> ClientResult<Option<WireChangeEvent>> {
        let Some(stream) = self.watch.as_mut() else {
            return Err(ClientError::Unsupported(
                "watch subscription is not active".into(),
            ));
        };
        match stream.message().await? {
            Some(event) => Ok(Some(change_event(event))),
            None => Ok(None),
        }
    }
}

fn telemetry(reply: grpc::TelemetryReply) -> WireTelemetrySnapshot {
    WireTelemetrySnapshot {
        writes_total: reply.writes_total,
        reads_total: reply.reads_total,
        write_conflicts_total: reply.write_conflicts_total,
        compactions_total: reply.compactions_total,
        fsyncs_total: reply.fsyncs_total,
        backups_completed_total: reply.backups_completed_total,
        backups_failed_total: reply.backups_failed_total,
        last_backup_timestamp_ms: reply.last_backup_timestamp_ms,
        last_backup_seq: reply.last_backup_seq,
        wal_segments_shipped_total: reply.wal_segments_shipped_total,
    }
}

fn change_event(event: grpc::ChangeEvent) -> WireChangeEvent {
    match event.kind {
        1 => WireChangeEvent::Delete {
            scope_id: event.scope_id,
            key: Bytes::from(event.key),
            seq: event.seq,
        },
        2 => WireChangeEvent::DropScope {
            scope_id: event.scope_id,
            seq: event.seq,
        },
        3 => WireChangeEvent::InvalidateScope {
            scope_id: event.scope_id,
            seq: event.seq,
        },
        _ => WireChangeEvent::Put {
            scope_id: event.scope_id,
            key: Bytes::from(event.key),
            value: Bytes::from(event.value),
            seq: event.seq,
        },
    }
}

fn pairs(entries: Vec<Pair>) -> Vec<(Bytes, Bytes)> {
    entries
        .into_iter()
        .map(|pair| (Bytes::from(pair.key), Bytes::from(pair.value)))
        .collect()
}

fn mutation_to_proto(scope_id: u32, key: Bytes, mutation: WireTxMutation) -> Mutation {
    const PUT: u32 = 0;
    const DELETE: u32 = 1;
    const MERGE: u32 = 2;
    const DELETE_RANGE: u32 = 3;
    match mutation {
        WireTxMutation::Put { val, expires_at_ms } => Mutation {
            scope_id,
            key: key.to_vec(),
            value: val.to_vec(),
            expires_at_ms,
            kind: PUT,
            end_key: Vec::new(),
        },
        WireTxMutation::Delete => Mutation {
            scope_id,
            key: key.to_vec(),
            value: Vec::new(),
            expires_at_ms: 0,
            kind: DELETE,
            end_key: Vec::new(),
        },
        WireTxMutation::Merge { operand } => Mutation {
            scope_id,
            key: key.to_vec(),
            value: operand.to_vec(),
            expires_at_ms: 0,
            kind: MERGE,
            end_key: Vec::new(),
        },
        WireTxMutation::DeleteRange { end_key } => Mutation {
            scope_id,
            key: key.to_vec(),
            value: Vec::new(),
            expires_at_ms: 0,
            kind: DELETE_RANGE,
            end_key: end_key.to_vec(),
        },
        _ => Mutation {
            scope_id,
            key: key.to_vec(),
            value: Vec::new(),
            expires_at_ms: 0,
            kind: DELETE,
            end_key: Vec::new(),
        },
    }
}

fn scope_options(options: WireScopeOptions) -> grpc::ScopeOptions {
    let (durability, sync_interval_ms) = match options.durability {
        WireDurability::Buffered { sync_interval } => (1, sync_interval.as_millis() as u64),
        _ => (0, 0),
    };
    let (retention, default_ttl_ms, compaction_style) = match options.retention {
        WireRetention::Ttl {
            default_ttl,
            compaction_style,
        } => (
            1,
            default_ttl.as_millis() as u64,
            match compaction_style {
                WireCompactionStyle::Fifo => 1,
                _ => 0,
            },
        ),
        _ => (0, 0, 0),
    };
    let prefix_bloom_len = options
        .prefix_bloom_len
        .and_then(|len| u32::try_from(len).ok())
        .unwrap_or(0);
    grpc::ScopeOptions {
        durability,
        sync_interval_ms,
        retention,
        default_ttl_ms,
        compaction_style,
        prefix_bloom_len,
        has_prefix_bloom: options.prefix_bloom_len.is_some(),
    }
}
