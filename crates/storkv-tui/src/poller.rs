use storkv_client::{AsyncClient, ClientError};

use crate::model::{AdminPoll, DashboardSnapshot, TelemetryPoll};

/// Fetch global telemetry from a live `storkvd` admin socket.
pub async fn fetch_telemetry(address: &str) -> Result<TelemetryPoll, ClientError> {
    let client = AsyncClient::connect_any(address).await?;
    let snapshot = client.telemetry().await?;
    Ok(TelemetryPoll::from(snapshot))
}

/// Fetch rich admin statistics (levels, per-scope metrics, latency p99).
pub async fn fetch_admin_stats(address: &str) -> Result<AdminPoll, ClientError> {
    let client = AsyncClient::connect_any(address).await?;
    let stats = client.admin_stats().await?;
    Ok(AdminPoll::from(stats))
}

/// Poll both telemetry and admin statistics over a single connection.
pub async fn fetch_dashboard(address: &str) -> Result<DashboardSnapshot, ClientError> {
    let client = AsyncClient::connect_any(address).await?;
    let telemetry = TelemetryPoll::from(client.telemetry().await?);
    let admin = AdminPoll::from(client.admin_stats().await?);
    Ok(DashboardSnapshot { telemetry, admin })
}
