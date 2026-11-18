//! JSONL import/export and Redis-to-StorKV migration.

use std::error::Error;
use std::path::Path;

use base64::Engine;
use serde::{Deserialize, Serialize};
use storkv_client::AsyncClient;

use crate::CliRunner;

mod import;
mod jsonl;
mod redis;

use import::apply_records;
use jsonl::read_records;
use redis::read_redis;

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    key_b64: String,
    value_b64: String,
    #[serde(default)]
    expires_at_ms: u64,
}

/// Destination of a migration.
pub enum MigrateTarget<'a> {
    /// Open the data directory directly (offline import).
    DataDir(&'a Path),
    /// Send writes to a running daemon over any supported transport.
    Daemon(&'a str),
}

impl CliRunner {
    /// Export every key in `scope` to JSONL and return the record count.
    pub async fn migrate_export(
        socket: &str,
        scope_name: &str,
        output: &Path,
    ) -> Result<usize, Box<dyn Error>> {
        let client = AsyncClient::connect_uri(socket).await?;
        let scope = import::open_online_scope(&client, scope_name).await?;
        let pairs = client.scan(&scope, None::<&[u8]>, None::<&[u8]>).await?;
        let mut encoded = String::new();
        for (key, value) in &pairs {
            let meta = client.key_meta(&scope, key).await?;
            let record = Record {
                key_b64: encode(key),
                value_b64: encode(value),
                expires_at_ms: meta.map(|meta| meta.expires_at_ms).unwrap_or(0),
            };
            encoded.push_str(&serde_json::to_string(&record)?);
            encoded.push('\n');
        }
        std::fs::write(output, encoded)?;
        Ok(pairs.len())
    }

    /// Import JSONL into a data directory or a running daemon.
    pub async fn migrate_import(
        target: MigrateTarget<'_>,
        scope_name: &str,
        input: &Path,
    ) -> Result<usize, Box<dyn Error>> {
        let records = read_records(input).await?;
        apply_records(target, scope_name, &records).await
    }

    /// Copy every key from a Redis-compatible server into StorKV.
    pub async fn migrate_from_redis(
        redis_url: &str,
        target: MigrateTarget<'_>,
        scope_name: &str,
    ) -> Result<usize, Box<dyn Error>> {
        let records = read_redis(redis_url).await?;
        apply_records(target, scope_name, &records).await
    }
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(text).ok()
}
