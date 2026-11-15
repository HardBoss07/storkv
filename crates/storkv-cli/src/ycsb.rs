use std::time::Instant;

use storkv_client::{AsyncClient, ClientError, ScopeHandle};

use crate::CliRunner;

const DEFAULT_SCOPE_NAME: &str = "default";

#[derive(Debug, Clone)]
pub struct YcsbSummary {
    pub workload: String,
    pub total_ops: usize,
    pub elapsed_ms: u64,
    pub ops_per_sec: f64,
}

impl CliRunner {
    pub async fn run_ycsb(
        socket: &str,
        workload: &str,
        record_count: usize,
        operation_count: usize,
    ) -> Result<YcsbSummary, ClientError> {
        let client = AsyncClient::connect_any(socket).await?;
        let scope = ScopeHandle {
            scope_id: 0,
            name: DEFAULT_SCOPE_NAME.into(),
        };

        let start_time = Instant::now();
        load_records(&client, &scope, record_count).await?;
        execute_workload(&client, &scope, workload, record_count, operation_count).await?;
        let elapsed = start_time.elapsed();

        Ok(YcsbSummary {
            workload: workload.to_string(),
            total_ops: record_count + operation_count,
            elapsed_ms: elapsed.as_millis() as u64,
            ops_per_sec: (record_count + operation_count) as f64 / elapsed.as_secs_f64(),
        })
    }
}

async fn load_records(
    client: &AsyncClient,
    scope: &ScopeHandle,
    record_count: usize,
) -> Result<(), ClientError> {
    for index in 0..record_count {
        let key = format!("user{:08}", index);
        let value = format!("val_{}_{}", index, "payload_padding_data_for_ycsb");
        client.put(scope, key.as_bytes(), value.as_bytes()).await?;
    }
    Ok(())
}

async fn execute_workload(
    client: &AsyncClient,
    scope: &ScopeHandle,
    workload: &str,
    record_count: usize,
    operation_count: usize,
) -> Result<(), ClientError> {
    for index in 0..operation_count {
        let target_key = format!("user{:08}", index % record_count);
        match workload {
            "a" => execute_workload_a(client, scope, &target_key, index).await?,
            "b" => execute_workload_b(client, scope, &target_key, index).await?,
            _ => read_one(client, scope, &target_key).await?,
        }
    }
    Ok(())
}

async fn execute_workload_a(
    client: &AsyncClient,
    scope: &ScopeHandle,
    target_key: &str,
    index: usize,
) -> Result<(), ClientError> {
    if index % 2 == 0 {
        read_one(client, scope, target_key).await
    } else {
        update_one(client, scope, target_key, index).await
    }
}

async fn execute_workload_b(
    client: &AsyncClient,
    scope: &ScopeHandle,
    target_key: &str,
    index: usize,
) -> Result<(), ClientError> {
    if index % 20 != 0 {
        read_one(client, scope, target_key).await
    } else {
        update_one(client, scope, target_key, index).await
    }
}

async fn read_one(client: &AsyncClient, scope: &ScopeHandle, key: &str) -> Result<(), ClientError> {
    let _ = client.get(scope, key.as_bytes()).await?;
    Ok(())
}

async fn update_one(
    client: &AsyncClient,
    scope: &ScopeHandle,
    key: &str,
    index: usize,
) -> Result<(), ClientError> {
    let new_value = format!("updated_val_{}", index);
    client
        .put(scope, key.as_bytes(), new_value.as_bytes())
        .await
}
