use crate::error::StorkvError;
use bytes::Bytes;
use crossbeam_channel::{Receiver, Sender, TrySendError};
use parking_lot::RwLock;

/// Maximum number of queued watch events for one subscriber.
pub const WATCH_CHANNEL_CAPACITY: usize = 1024;
/// Maximum number of concurrent watch subscribers.
pub const MAX_WATCH_SUBSCRIBERS: usize = 1024;
/// Maximum number of events replayed for one resume request.
pub const MAX_REPLAY_EVENTS: usize = 1024;
/// Maximum total payload bytes replayed for one resume request.
pub const MAX_REPLAY_BYTES: usize = 16 * 1024 * 1024;

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
    /// A scope needs a full cache invalidation.
    ///
    /// The engine sends this event for a range delete or a merge. The change
    /// can touch many keys, so the event does not name a key.
    InvalidateScope {
        /// Commit sequence number of the change.
        seq: u64,
        /// Identifier of the affected scope.
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
            ChangeEvent::InvalidateScope { seq, .. } => *seq,
        }
    }

    /// Return the identifier of the changed scope.
    pub fn scope_id(&self) -> u32 {
        match self {
            ChangeEvent::Put { scope_id, .. } => *scope_id,
            ChangeEvent::Delete { scope_id, .. } => *scope_id,
            ChangeEvent::DropScope { scope_id, .. } => *scope_id,
            ChangeEvent::InvalidateScope { scope_id, .. } => *scope_id,
        }
    }

    pub(crate) fn payload_bytes(&self) -> usize {
        match self {
            ChangeEvent::Put { key, value, .. } => key.len() + value.len(),
            ChangeEvent::Delete { key, .. } => key.len(),
            ChangeEvent::DropScope { .. } => 0,
            ChangeEvent::InvalidateScope { .. } => 0,
        }
    }
}

/// Registry of live watch subscribers.
pub struct WatchRegistry {
    subscribers: RwLock<Vec<Sender<ChangeEvent>>>,
}

impl Default for WatchRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            subscribers: RwLock::new(Vec::new()),
        }
    }

    /// Add a subscriber and return its event receiver.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::TooManySubscribers`] when the registry is full.
    pub fn subscribe(&self) -> crate::Result<Receiver<ChangeEvent>> {
        self.subscribe_replaying(Vec::new())
    }

    /// Add a subscriber that first receives the given replay events.
    ///
    /// The caller must hold the commit lock. This makes the replay-to-live
    /// transition atomic and prevents lost or duplicated events.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::ReplayBudgetExceeded`] when the replay is larger
    /// than the queue or the replay budget. Returns
    /// [`StorkvError::TooManySubscribers`] when the registry is full.
    pub fn subscribe_replaying(
        &self,
        replay: Vec<ChangeEvent>,
    ) -> crate::Result<Receiver<ChangeEvent>> {
        let events = replay.len();
        let bytes = replay.iter().map(ChangeEvent::payload_bytes).sum();
        if events > MAX_REPLAY_EVENTS || bytes > MAX_REPLAY_BYTES {
            return Err(StorkvError::ReplayBudgetExceeded { events, bytes });
        }
        let (sender, receiver) = crossbeam_channel::bounded(WATCH_CHANNEL_CAPACITY);
        for event in replay {
            if sender.try_send(event).is_err() {
                return Err(StorkvError::ReplayBudgetExceeded { events, bytes });
            }
        }
        let mut subscribers = self.subscribers.write();
        if subscribers.len() >= MAX_WATCH_SUBSCRIBERS {
            return Err(StorkvError::TooManySubscribers {
                count: subscribers.len(),
                max: MAX_WATCH_SUBSCRIBERS,
            });
        }
        subscribers.push(sender);
        Ok(receiver)
    }

    /// Send one event to every live subscriber.
    ///
    /// A subscriber that cannot accept the event is removed. Its receiver then
    /// observes a disconnect and its worker task stops.
    pub fn broadcast(&self, event: ChangeEvent) {
        let mut subscribers = self.subscribers.write();
        subscribers.retain(|sender| match sender.try_send(event.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
        });
    }
}
