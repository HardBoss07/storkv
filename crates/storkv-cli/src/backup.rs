use storkv_client::{AsyncClient, ClientError};

use crate::CliRunner;

impl CliRunner {
    pub async fn backup(socket: &str, output_dir: &str) -> Result<(), ClientError> {
        let client = AsyncClient::connect_any(socket).await?;
        client.checkpoint(output_dir).await?;
        Ok(())
    }
}
