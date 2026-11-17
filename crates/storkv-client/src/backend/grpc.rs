use bytes::Bytes;
use storkv_proto::{
    Request, Response, WireBackupEntry, WireBackupKind, WireBackupReport, WireBackupStatus,
    WireBackupVerifyError, WireBackupVerifyReport, WireChangeEvent, WireCompactionStyle,
    WireDurability, WireMergeOp, WireRetention, WireScopeHandle, WireScopeOptions,
    WireTelemetrySnapshot, WireTxMutation,
};
use tonic::Streaming;
use tonic::transport::Channel;

use crate::grpc::{
    self, Empty, GetRequest, MergeRequest, Mutation, Pair, PutRequest, ReadEntry, ScanRequest,
    ScopeRef, stor_kv_admin_client::StorKvAdminClient, stor_kv_client::StorKvClient,
};
use crate::transport::{ClientError, ClientResult};

/// gRPC backend using the generated `storkv.v1` clients over h2c.
pub(crate) struct GrpcBackend {
    client: StorKvClient<Channel>,
    admin: StorKvAdminClient<Channel>,
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
            client: StorKvClient::new(channel.clone()),
            admin: StorKvAdminClient::new(channel),
            watch: None,
        })
    }

    pub(crate) async fn send(&mut self, request: Request) -> ClientResult<Response> {
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
                    .delete(grpc::DeleteRequest {
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
                    .delete_range(grpc::DeleteRangeRequest {
                        scope_id,
                        start_key: start_key.to_vec(),
                        end_key: end_key.to_vec(),
                    })
                    .await?;
                Response::Empty
            }
            Request::Merge { scope_id, key, op } => {
                let (counter_delta, custom) = match op {
                    WireMergeOp::CounterAdd(delta) => (delta, Vec::new()),
                    WireMergeOp::Custom(bytes) => (0, bytes.to_vec()),
                    _ => (0, Vec::new()),
                };
                self.client
                    .merge(MergeRequest {
                        scope_id,
                        key: key.to_vec(),
                        counter_delta,
                        custom,
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
                    Response::Error(storkv_proto::StorKvWireError::InvalidOperation(
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
            Request::CreateSnapshot { name } => {
                self.admin
                    .create_snapshot(ScopeRef { scope_id: 0, name })
                    .await?;
                Response::Empty
            }
            Request::UnpinSnapshot { name } => {
                self.admin
                    .unpin_snapshot(ScopeRef { scope_id: 0, name })
                    .await?;
                Response::Empty
            }
            Request::Checkpoint { dest_dir } => {
                self.admin
                    .checkpoint(grpc::CheckpointRequest { dest_dir })
                    .await?;
                Response::Empty
            }
            Request::Compact => {
                self.admin.compact(Empty {}).await?;
                Response::Empty
            }
            Request::AdminStats => {
                let reply = self.admin.admin_stats(Empty {}).await?.into_inner();
                Response::Telemetry(telemetry(reply))
            }
            Request::TriggerBackup { wait } => {
                let reply = self
                    .admin
                    .trigger_backup(grpc::BackupRequest { wait })
                    .await?
                    .into_inner();
                Response::BackupReport(WireBackupReport {
                    generation_id: reply.generation_id,
                    sequence: reply.seq,
                    ..WireBackupReport::default()
                })
            }
            Request::BackupStatus => {
                let reply = self.admin.backup_status(Empty {}).await?.into_inner();
                Response::BackupStatus(WireBackupStatus {
                    enabled: reply.enabled,
                    last_backup_timestamp_ms: reply.last_backup_timestamp_ms,
                    last_backup_seq: reply.last_backup_seq,
                    backups_completed_total: reply.backups_completed_total,
                    backups_failed_total: reply.backups_failed_total,
                    wal_segments_shipped_total: reply.wal_segments_shipped_total,
                    last_error: (!reply.last_error.is_empty()).then_some(reply.last_error),
                    backup_root: (!reply.backup_root.is_empty()).then_some(reply.backup_root),
                    next_full_backup_in_ms: reply.next_full_backup_in_ms,
                    sink: reply.sink,
                })
            }
            Request::VerifyBackup { target_seq } => {
                let reply = self
                    .admin
                    .verify_backup(grpc::VerifyBackupRequest {
                        has_target_seq: target_seq.is_some(),
                        target_seq: target_seq.unwrap_or(0),
                    })
                    .await?
                    .into_inner();
                Response::BackupVerifyReport(WireBackupVerifyReport {
                    generations_checked: reply.generations_checked,
                    files_checked: reply.files_checked,
                    bytes_checked: reply.bytes_checked,
                    wal_segments_checked: 0,
                    errors: reply
                        .errors
                        .into_iter()
                        .map(|detail| WireBackupVerifyError {
                            path: String::new(),
                            detail,
                        })
                        .collect(),
                })
            }
            Request::ListBackups => {
                let reply = self.admin.list_backups(Empty {}).await?.into_inner();
                Response::BackupCatalog(
                    reply
                        .entries
                        .into_iter()
                        .map(|entry| WireBackupEntry {
                            generation_id: entry.generation_id,
                            kind: match entry.kind {
                                2 => WireBackupKind::Wal,
                                _ => WireBackupKind::Full,
                            },
                            created_at_ms: entry.created_at_ms,
                            min_seq: entry.min_seq,
                            max_seq: entry.max_seq,
                            total_bytes: entry.total_bytes,
                            files: entry.files,
                            compression: entry.compression,
                        })
                        .collect(),
                )
            }
            Request::WatchSubscribe => {
                self.start_watch().await?;
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

    pub(crate) async fn start_watch(&mut self) -> ClientResult<()> {
        let stream = self
            .client
            .watch(grpc::WatchRequest {
                scope_id: 0,
                all_scopes: true,
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
