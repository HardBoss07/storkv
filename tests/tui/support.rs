use crate::common;
use std::sync::Arc;
use storkv_server::Server;

pub(super) async fn spawn_test_server() -> (Arc<Server>, String, crate::common::TempDb) {
    let mut temp_db = common::TempDb::new("tui_server");
    let server = Arc::new(Server::with_db(Arc::new(temp_db.db.take().unwrap())));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr").to_string();

    let server_clone = server.clone();
    tokio::spawn(async move {
        let _ = server_clone.run_tcp_listener(listener, true).await;
    });

    (server, addr, temp_db)
}
