use storkv_client::{AsyncClient, ClientError};

use crate::CliRunner;

impl CliRunner {
    pub async fn health(socket: &str) -> Result<(), ClientError> {
        let client = AsyncClient::connect_any(socket).await?;
        client.health_check().await?;
        Ok(())
    }
}
