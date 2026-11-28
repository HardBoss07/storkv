//! Subscription workers and event filtering for watch streams.

mod map;

use map::{map_core_event, map_wire_event};

use std::time::Duration;

use bytes::Bytes;
use storkv_client::AsyncClient;
use storkv_core::Db;
use tokio::sync::mpsc;

use crate::error::Result;

use super::WatchStream;

const WATCH_BUFFER: usize = 256;
const IDLE_POLL: Duration = Duration::from_millis(100);
const RECONNECT_MIN: Duration = Duration::from_millis(25);
const RECONNECT_MAX: Duration = Duration::from_secs(2);

pub(crate) fn spawn_embedded(
    db: Db,
    scope_id: u32,
    prefix: Bytes,
    from_seq: Option<u64>,
) -> Result<WatchStream> {
    let (sender, receiver) = mpsc::channel(WATCH_BUFFER);
    let events = match from_seq {
        Some(seq) => db.watch_from(seq)?,
        None => db.watch()?,
    };
    tokio::task::spawn_blocking(move || {
        loop {
            if sender.is_closed() {
                break;
            }
            match events.recv_timeout(IDLE_POLL) {
                Ok(event) => {
                    if let Some(mapped) = map_core_event(event, scope_id, &prefix)
                        && sender.blocking_send(Ok(mapped)).is_err()
                    {
                        break;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    Ok(WatchStream::new(receiver))
}

pub(crate) fn spawn_networked(
    uri: String,
    scope_id: u32,
    prefix: Bytes,
    from_seq: Option<u64>,
) -> WatchStream {
    let (sender, receiver) = mpsc::channel(WATCH_BUFFER);
    let (ready_sender, ready_receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let mut ready_sender = Some(ready_sender);
        let mut delay = RECONNECT_MIN;
        loop {
            if sender.is_closed() {
                return;
            }
            let client = match AsyncClient::connect_uri(&uri).await {
                Ok(client) => client,
                Err(_) => {
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(RECONNECT_MAX);
                    continue;
                }
            };
            let mut stream = match client.subscribe_watch_from(from_seq).await {
                Ok(stream) => stream,
                Err(_) => {
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(RECONNECT_MAX);
                    continue;
                }
            };
            if let Some(sender) = ready_sender.take() {
                let _ = sender.send(());
            }
            loop {
                match stream.next().await {
                    Ok(Some(event)) => {
                        if let Some(mapped) = map_wire_event(event, scope_id, &prefix)
                            && sender.send(Ok(mapped)).await.is_err()
                        {
                            return;
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
            if sender.is_closed() {
                return;
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(RECONNECT_MAX);
        }
    });
    WatchStream::with_ready(receiver, ready_receiver)
}
