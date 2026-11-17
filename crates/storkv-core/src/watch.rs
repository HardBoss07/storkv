use bytes::Bytes;
use crossbeam_channel::{Receiver, Sender};
use parking_lot::RwLock;

/// Change notification sent to watch subscribers.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChangeEvent {
    /// A key received a new value.
    Put {
        /// New value bytes.
        value: Bytes,
        /// User key that changed.
        key: Bytes,
        /// Commit sequence number of the change.
        seq: u64,
        /// Identifier of the scope.
        scope_id: u32,
    },
    /// A key was deleted.
    Delete {
        /// User key that was deleted.
        key: Bytes,
        /// Commit sequence number of the change.
        seq: u64,
        /// Identifier of the scope.
        scope_id: u32,
    },
    /// A scope was dropped.
    DropScope {
        /// Commit sequence number of the change.
        seq: u64,
        /// Identifier of the dropped scope.
        scope_id: u32,
    },
}

const _: () = assert!(std::mem::size_of::<ChangeEvent>() <= 96);

impl ChangeEvent {
    /// Return the commit sequence number of the event.
    pub fn seq(&self) -> u64 {
        match self {
            ChangeEvent::Put { seq, .. } => *seq,
            ChangeEvent::Delete { seq, .. } => *seq,
            ChangeEvent::DropScope { seq, .. } => *seq,
        }
    }
}

/// Registry of watch subscribers.
pub struct WatchRegistry {
    subscribers: RwLock<Vec<Sender<ChangeEvent>>>,
}

impl WatchRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            subscribers: RwLock::new(Vec::new()),
        }
    }

    /// Add a subscriber and return its event receiver.
    pub fn subscribe(&self) -> Receiver<ChangeEvent> {
        let (tx, rx) = crossbeam_channel::unbounded();
        self.subscribers.write().push(tx);
        rx
    }

    /// Send one event to every live subscriber.
    pub fn broadcast(&self, event: ChangeEvent) {
        let mut subs = self.subscribers.write();
        subs.retain(|sender| sender.send(event.clone()).is_ok());
    }
}
