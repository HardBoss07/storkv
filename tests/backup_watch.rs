//! Backup, checkpoint, watch, and telemetry integration tests.
//!
//! Bodies live in `tests/backup_watch/`; this file only wires the modules
//! together so the existing test names and binary stay stable.

mod common;

#[path = "backup_watch/chaos.rs"]
mod chaos;
#[path = "backup_watch/checkpoint.rs"]
mod checkpoint;
#[path = "backup_watch/local_sink.rs"]
mod local_sink;
#[path = "backup_watch/pipe_sink.rs"]
mod pipe_sink;
#[path = "backup_watch/restore.rs"]
mod restore;
#[path = "backup_watch/support.rs"]
mod support;
#[path = "backup_watch/telemetry.rs"]
mod telemetry;
#[path = "backup_watch/verify.rs"]
mod verify;
#[path = "backup_watch/watch.rs"]
mod watch;
