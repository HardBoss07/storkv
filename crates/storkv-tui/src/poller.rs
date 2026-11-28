//! Async pollers for live telemetry and admin statistics.

use storkv_client::{AsyncClient, ClientError};

use crate::model::{AdminPoll, BackupPoll, DashboardSnapshot, TelemetryPoll};

/// Fetch global telemetry from a live `storkvd` admin socket.
///
/// # Errors
///
/// Returns an error when the connection or the telemetry request fails.
pub async fn fetch_telemetry(address: &str) -> Result<TelemetryPoll, ClientError> {
    let client = AsyncClient::connect_uri(address).await?;
    let snapshot = client.telemetry().await?;
    Ok(TelemetryPoll::from(snapshot))
}

/// Fetch rich admin statistics (levels, per-scope metrics, latency p99).
///
/// # Errors
///
/// Returns an error when the connection or the admin request fails.
pub async fn fetch_admin_stats(address: &str) -> Result<AdminPoll, ClientError> {
    let client = AsyncClient::connect_uri(address).await?;
    let stats = client.admin_stats().await?;
    Ok(AdminPoll::from(stats))
}

/// Poll both telemetry and admin statistics over a single connection.
///
/// # Errors
///
/// Returns an error when the connection, telemetry, admin, or backup request
/// fails.
pub async fn fetch_dashboard(address: &str) -> Result<DashboardSnapshot, ClientError> {
    let client = AsyncClient::connect_uri(address).await?;
    let telemetry = TelemetryPoll::from(client.telemetry().await?);
    let admin = AdminPoll::from(client.admin_stats().await?);
    let backup = BackupPoll::from(client.backup_status().await?);
    Ok(DashboardSnapshot {
        telemetry,
        admin,
        backup,
    })
}

/// Fetch only the autonomous backup status.
///
/// # Errors
///
/// Returns an error when the connection or the backup status request fails.
pub async fn fetch_backup_status(address: &str) -> Result<BackupPoll, ClientError> {
    let client = AsyncClient::connect_uri(address).await?;
    Ok(BackupPoll::from(client.backup_status().await?))
}
