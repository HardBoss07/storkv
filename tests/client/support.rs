use crate::common;
use std::sync::Arc;
use storkv_server::Server;
use tokio::net::TcpListener;

pub(super) async fn spawn_test_server(
    prefix: &str,
) -> (Arc<Server>, String, crate::common::TempDb) {
    let mut temp_db = common::TempDb::new(prefix);
    let server = Arc::new(Server::with_db(Arc::new(temp_db.db.take().unwrap())));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let server_clone = server.clone();
    tokio::spawn(async move {
        server_clone
            .run_tcp_listener(listener, false)
            .await
            .unwrap();
    });

    (server, addr, temp_db)
}
