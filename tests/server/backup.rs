use std::sync::Arc;

use storkv_core::backup::BackupConfig;
use storkv_proto::{Request, Response};
use storkv_server::Server;
use tokio::net::{TcpListener, TcpStream};

use super::support::send_request;

async fn spawn_with_backup(
    prefix: &str,
) -> (
    Arc<Server>,
    String,
    crate::common::TempDb,
    std::path::PathBuf,
) {
    let mut temp_db = crate::common::TempDb::new(prefix);
    let backup_root = temp_db.path.join("backups");
    let server = Arc::new(Server::with_db(Arc::new(temp_db.db.take().unwrap())));
    server
        .db()
        .start_auto_backup(BackupConfig::local(backup_root.clone()))
        .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let server_clone = server.clone();
    tokio::spawn(async move {
        let _ = server_clone.run_tcp_listener(listener, false).await;
    });

    (server, addr, temp_db, backup_root)
}

#[tokio::test]
async fn backup_opcodes_round_trip_over_native_transport() {
    let (server, addr, _temp_db, backup_root) = spawn_with_backup("server_backup_wire").await;
    let mut stream = TcpStream::connect(&addr).await.unwrap();

    assert_eq!(
        send_request(
            &mut stream,
            1,
            Request::Put {
                scope_id: 0,
                key: bytes::Bytes::from_static(b"k"),
                val: bytes::Bytes::from_static(b"v"),
            },
        )
        .await,
        Response::Empty
    );

    let report = match send_request(&mut stream, 2, Request::TriggerBackup { wait: true }).await {
        Response::BackupReport(report) => report,
        other => panic!("expected backup report, got {other:?}"),
    };
    assert!(!report.generation_id.is_empty());
    assert!(report.sequence >= 1);

    let status = match send_request(&mut stream, 3, Request::BackupStatus).await {
        Response::BackupStatus(status) => status,
        other => panic!("expected backup status, got {other:?}"),
    };
    assert!(status.enabled);
    assert_eq!(status.backup_root.as_deref(), backup_root.to_str());
    assert!(status.backups_completed_total >= 1);

    let catalog = match send_request(&mut stream, 4, Request::ListBackups).await {
        Response::BackupCatalog(entries) => entries,
        other => panic!("expected backup catalog, got {other:?}"),
    };
    assert!(
        catalog
            .iter()
            .any(|entry| entry.generation_id == report.generation_id)
    );

    let verification =
        match send_request(&mut stream, 5, Request::VerifyBackup { target_seq: None }).await {
            Response::BackupVerifyReport(report) => report,
            other => panic!("expected verification report, got {other:?}"),
        };
    assert!(verification.is_ok(), "errors: {:?}", verification.errors);
    assert!(verification.generations_checked >= 1);

    let telemetry = match send_request(&mut stream, 6, Request::TelemetryQuery).await {
        Response::Telemetry(snapshot) => snapshot,
        other => panic!("expected telemetry, got {other:?}"),
    };
    assert!(telemetry.backups_completed_total >= 1);
    assert!(telemetry.last_backup_seq >= 1);

    server.db().stop_auto_backup().unwrap();
    let _ = server.shutdown_handle().send(());
}

#[tokio::test]
async fn backup_status_without_configuration_is_disabled() {
    let (addr, server, _temp_db, _handle) = super::support::spawn_server("server_backup_off").await;
    let mut stream = TcpStream::connect(&addr).await.unwrap();

    match send_request(&mut stream, 1, Request::BackupStatus).await {
        Response::BackupStatus(status) => {
            assert!(!status.enabled);
            assert!(status.backup_root.is_none());
        }
        other => panic!("expected disabled status, got {other:?}"),
    }

    let _ = server.shutdown_handle().send(());
}
