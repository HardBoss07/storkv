//! Active TTL expiration pass.

use std::cmp::Reverse;
use std::sync::atomic::Ordering;

use bytes::Bytes;

use crate::db::DbInner;
use crate::format::{InternalKey, ValueType};
use crate::scope::StorageMode;
use crate::types::{SequenceNumber, TimestampMs};
use crate::vfs::FileSystem;
use crate::wal::{WalBatchFrame, WalOp};
use crate::watch::ChangeEvent;

impl<FS: FileSystem + 'static> DbInner<FS> {
    /// Pop expired deadlines across active scopes with active TTL tracking,
    /// verify their metadata, emit [`ChangeEvent::Expired`], and append a
    /// tombstone.
    ///
    /// # Errors
    ///
    /// Returns an error when writing the WAL or reading key metadata fails.
    pub fn expire_due_keys(&self) -> crate::Result<usize> {
        let now_ms = TimestampMs::now().get();
        let scopes: Vec<_> = self.scopes.read().values().cloned().collect();
        let mut expired_total = 0;

        for scope in scopes {
            if !scope.guardrails.read().active_ttl_tracking || scope.is_dropped() {
                continue;
            }
            let mut due = Vec::new();
            {
                let mut heap = scope.ttl_tracker.lock();
                while let Some(Reverse((deadline, _))) = heap.peek() {
                    if *deadline <= now_ms {
                        if let Some(Reverse((deadline, key))) = heap.pop() {
                            due.push((deadline, key));
                        }
                    } else {
                        break;
                    }
                }
            }

            for (deadline, key) in due {
                let read_seq = self.last_visible_seq.load(Ordering::Acquire);
                let meta =
                    scope.key_meta(&key, SequenceNumber::new(read_seq), TimestampMs::now())?;
                let Some(meta) = meta else {
                    continue;
                };
                if meta.expires_at_ms != deadline {
                    continue;
                }

                let _guard = self.commit_lock.lock();
                let op_seq = self.last_visible_seq.fetch_add(1, Ordering::SeqCst) + 1;
                let internal_key = InternalKey::new(key.clone(), op_seq, ValueType::Tombstone, 0);
                scope
                    .active_memtable
                    .read()
                    .insert(internal_key.encode(), Bytes::new(), op_seq);

                let wal_op = WalOp::Delete {
                    scope_id: scope.scope_id,
                    key: key.clone(),
                };
                let frame = WalBatchFrame::new(op_seq, vec![wal_op]);
                if scope.options.storage_mode != StorageMode::InMemory {
                    self.wal_mgr.write_batch(&frame, scope.options.durability)?;
                }
                self.watch_registry.broadcast(ChangeEvent::Expired {
                    scope_id: scope.scope_id,
                    key,
                    seq: op_seq,
                });
                self.replication_registry.broadcast(frame);
                expired_total += 1;
            }
        }
        Ok(expired_total)
    }
}
