//! Daemon health check subcommand.
use storkv_client::{AsyncClient, ClientError};

use crate::CliRunner;

impl CliRunner {
    /// Run a health check against a live server.
    ///
    /// # Errors
    ///
    /// Returns a client error when the connection or check fails.
    pub async fn health(socket: &str) -> Result<(), ClientError> {
        let client = AsyncClient::connect_uri(socket).await?;
        client.health_check().await?;
        Ok(())
    }
}
