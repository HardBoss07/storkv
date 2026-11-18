use storkv_client::{AsyncClient, ClientError};
use storkv_proto::{WireBackupStatus, WireBackupVerifyReport};

use crate::CliRunner;

impl CliRunner {
    /// Copy a consistent checkpoint to `output_dir`.
    ///
    /// # Errors
    ///
    /// Returns a client error when the connection or checkpoint fails.
    pub async fn backup(socket: &str, output_dir: &str) -> Result<(), ClientError> {
        let client = AsyncClient::connect_uri(socket).await?;
        client.checkpoint(output_dir).await?;
        Ok(())
    }

    /// Fetch the live backup worker status.
    ///
    /// # Errors
    ///
    /// Returns a client error when the connection or request fails.
    pub async fn backup_status(socket: &str) -> Result<WireBackupStatus, ClientError> {
        let client = AsyncClient::connect_uri(socket).await?;
        client.backup_status().await
    }

    /// Verify archived backup generations.
    ///
    /// # Errors
    ///
    /// Returns a client error when the connection or request fails.
    pub async fn backup_verify(
        socket: &str,
        target_seq: Option<u64>,
    ) -> Result<WireBackupVerifyReport, ClientError> {
        let client = AsyncClient::connect_uri(socket).await?;
        client.verify_backup(target_seq).await
    }
}
