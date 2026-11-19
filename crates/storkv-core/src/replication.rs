use crate::error::StorkvError;
use crate::wal::{WalBatchFrame, WalOp};
use crossbeam_channel::{Receiver, Sender, TrySendError};
use parking_lot::RwLock;

/// Maximum number of queued replication frames for one subscriber.
pub const REPLICATION_CHANNEL_CAPACITY: usize = 1024;
/// Maximum number of concurrent replication subscribers.
pub const MAX_REPLICATION_SUBSCRIBERS: usize = 1024;
/// Maximum number of frames replayed for one resume request.
pub const MAX_REPLICATION_REPLAY_EVENTS: usize = 1024;
/// Maximum total payload bytes replayed for one resume request.
pub const MAX_REPLICATION_REPLAY_BYTES: usize = 16 * 1024 * 1024;

/// Registry of replication frame subscribers.
///
/// A primary broadcasts each committed [`WalBatchFrame`] to every registered
/// replica. A replica that resumes from a cursor first receives the replayed
/// frames, then the live frames, without gaps.
pub struct ReplicationRegistry {
    subscribers: RwLock<Vec<Sender<WalBatchFrame>>>,
}

impl Default for ReplicationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplicationRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            subscribers: RwLock::new(Vec::new()),
        }
    }

    /// Add a subscriber and return its frame receiver.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::TooManySubscribers`] when the registry is full.
    pub fn subscribe(&self) -> crate::Result<Receiver<WalBatchFrame>> {
        self.subscribe_replaying(Vec::new())
    }

    /// Add a subscriber that first receives the given replay frames.
    ///
    /// The caller must hold the commit lock. This makes the replay-to-live
    /// transition atomic.
    ///
    /// # Errors
    ///
    /// Returns [`StorkvError::ReplayBudgetExceeded`] when the replay is larger
    /// than the queue or the replay budget. Returns
    /// [`StorkvError::TooManySubscribers`] when the registry is full.
    pub fn subscribe_replaying(
        &self,
        replay: Vec<WalBatchFrame>,
    ) -> crate::Result<Receiver<WalBatchFrame>> {
        let events = replay.len();
        let bytes = replay.iter().map(frame_payload_bytes).sum();
        if events > MAX_REPLICATION_REPLAY_EVENTS || bytes > MAX_REPLICATION_REPLAY_BYTES {
            return Err(StorkvError::ReplayBudgetExceeded { events, bytes });
        }
        let (sender, receiver) = crossbeam_channel::bounded(REPLICATION_CHANNEL_CAPACITY);
        for frame in replay {
            if sender.try_send(frame).is_err() {
                return Err(StorkvError::ReplayBudgetExceeded { events, bytes });
            }
        }
        let mut subscribers = self.subscribers.write();
        if subscribers.len() >= MAX_REPLICATION_SUBSCRIBERS {
            return Err(StorkvError::TooManySubscribers {
                count: subscribers.len(),
                max: MAX_REPLICATION_SUBSCRIBERS,
            });
        }
        subscribers.push(sender);
        Ok(receiver)
    }

    /// Send one frame to every live subscriber.
    ///
    /// A subscriber that cannot accept the frame is removed. Its receiver then
    /// observes a disconnect and its worker task stops.
    pub fn broadcast(&self, frame: WalBatchFrame) {
        let mut subscribers = self.subscribers.write();
        subscribers.retain(|sender| match sender.try_send(frame.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
        });
    }
}

pub(crate) fn frame_payload_bytes(frame: &WalBatchFrame) -> usize {
    frame.ops.iter().map(wal_op_payload_bytes).sum()
}

fn wal_op_payload_bytes(op: &WalOp) -> usize {
    match op {
        WalOp::Put { key, val, .. } => key.len() + val.len(),
        WalOp::Delete { key, .. } => key.len(),
        WalOp::Merge { key, val, .. } => key.len() + val.len(),
        WalOp::DeleteRange {
            start_key, end_key, ..
        } => start_key.len() + end_key.len(),
        WalOp::DropScope { .. } => 0,
    }
}
