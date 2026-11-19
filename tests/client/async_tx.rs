use bytes::Bytes;
use storkv_client::{AsyncClient, ClientError};
use storkv_proto::{StorkvWireError, WireDurability, WireRetention};

use super::support::spawn_test_server;

#[tokio::test]
async fn test_async_client_transaction_occ_and_ryow() {
    let (server, addr, _temp_db) = spawn_test_server("client_tx").await;
    let client1 = AsyncClient::connect(&addr).await.unwrap();
    let client2 = AsyncClient::connect(&addr).await.unwrap();

    let scope = client1
        .create_scope(
            "accounts",
            WireDurability::StrictFsync,
            WireRetention::Permanent,
            None,
        )
        .await
        .unwrap();

    client1.put(&scope, b"acc:1", b"100").await.unwrap();

    let mut transaction = client1.begin_tx().await.unwrap();

    let value = transaction.get(&scope, b"acc:1").await.unwrap();
    assert_eq!(value, Some(Bytes::from_static(b"100")));

    transaction.put(&scope, b"acc:1", b"150");
    let own_write_value = transaction.get(&scope, b"acc:1").await.unwrap();
    assert_eq!(own_write_value, Some(Bytes::from_static(b"150")));

    client2.put(&scope, b"acc:1", b"120").await.unwrap();

    let commit_error = transaction.commit().await.unwrap_err();
    match commit_error {
        ClientError::Server(StorkvWireError::WriteConflict { .. }) => {}
        other => panic!("Expected WriteConflict, got {:?}", other),
    }

    server.shutdown_handle().send(()).unwrap();
}
