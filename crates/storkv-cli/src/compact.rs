use storkv_client::{AsyncClient, ClientError};

use crate::CliRunner;

impl CliRunner {
    /// Run full compaction, or a range compaction when `scope` is set.
    ///
    /// # Errors
    ///
    /// Returns a client error when the connection or compaction fails.
    pub async fn compact(
        socket: &str,
        scope: Option<&str>,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
    ) -> Result<(), ClientError> {
        let client = AsyncClient::connect_uri(socket).await?;
        match scope {
            Some(scope) => client.compact_range(scope, start, end).await,
            None => client.compact().await,
        }
    }
}
