//! Validation and admin classification methods for protocol requests.

use super::enum_def::Request;

impl Request {
    /// Return true when the request mutates global state or exposes backups.
    ///
    /// The server honours these requests only on an admin connection when it
    /// runs in strict-admin mode.
    pub fn is_admin_only(&self) -> bool {
        matches!(
            self,
            Request::Checkpoint { .. }
                | Request::Compact
                | Request::CompactRange { .. }
                | Request::CreateSnapshot { .. }
                | Request::UnpinSnapshot { .. }
                | Request::DropScope { .. }
                | Request::TriggerBackup { .. }
                | Request::VerifyBackup { .. }
                | Request::ListBackups
                | Request::ReplicationStart { .. }
        )
    }

    /// Return true when the request mutates stored data or scope layout.
    ///
    /// Replicas reject these requests with [`crate::StorkvWireError::NotPrimary`].
    pub fn is_write(&self) -> bool {
        matches!(
            self,
            Request::Put { .. }
                | Request::PutWithTtl { .. }
                | Request::PutIf { .. }
                | Request::Delete { .. }
                | Request::DeleteRange { .. }
                | Request::Merge { .. }
                | Request::TxCommit { .. }
                | Request::DropScope { .. }
                | Request::CreateScope { .. }
        )
    }
}
