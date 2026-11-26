mod common;

#[path = "server/admin.rs"]
mod admin;
#[path = "server/backup.rs"]
mod backup;
#[path = "server/concurrency.rs"]
mod concurrency;
#[path = "server/config.rs"]
mod config;
#[path = "server/lifecycle.rs"]
mod lifecycle;
#[path = "server/put_if.rs"]
mod put_if;
#[path = "server/replication.rs"]
mod replication;
#[path = "server/support.rs"]
mod support;
#[path = "server/transactions.rs"]
mod transactions;
#[path = "server/transport.rs"]
mod transport;
#[path = "server/watch.rs"]
mod watch;
