use bytes::Bytes;
use crossbeam_channel::{Receiver, Sender};
use parking_lot::RwLock;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChangeEvent {
    Put {
        scope_id: u32,
        key: Bytes,
        value: Bytes,
        seq: u64,
    },
    Delete {
        scope_id: u32,
        key: Bytes,
        seq: u64,
    },
    DropScope {
        scope_id: u32,
        seq: u64,
    },
}

impl ChangeEvent {
    pub fn seq(&self) -> u64 {
        match self {
            ChangeEvent::Put { seq, .. } => *seq,
            ChangeEvent::Delete { seq, .. } => *seq,
            ChangeEvent::DropScope { seq, .. } => *seq,
        }
    }
}

pub struct WatchRegistry {
    subscribers: RwLock<Vec<Sender<ChangeEvent>>>,
}

impl WatchRegistry {
    pub fn new() -> Self {
        Self {
            subscribers: RwLock::new(Vec::new()),
        }
    }

    pub fn subscribe(&self) -> Receiver<ChangeEvent> {
        let (tx, rx) = crossbeam_channel::unbounded();
        self.subscribers.write().push(tx);
        rx
    }

    pub fn broadcast(&self, event: ChangeEvent) {
        let mut subs = self.subscribers.write();
        subs.retain(|sender| sender.send(event.clone()).is_ok());
    }
}
