//! SDK error type and retry classification helpers.

use storkv_client::ClientError;
use storkv_core::StorkvError;
use storkv_proto::StorkvWireError;
use thiserror::Error;

/// Result alias for StorKV SDK operations.
pub type Result<T> = std::result::Result<T, SdkError>;

/// Errors produced by the StorKV SDK.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SdkError {
    /// The embedded storage engine returned an error.
    #[error(transparent)]
    Core(#[from] StorkvError),
    /// The networked client returned an error.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// The pool could not hand out a connection before the acquire timeout.
    #[error("timed out while waiting for a pooled connection")]
    AcquireTimeout,
    /// The pool is closed.
    #[error("the connection pool is closed")]
    PoolClosed,
    /// The connection URI is not recognised by the SDK.
    #[error("invalid StorKV connection URI: {0}")]
    InvalidUri(String),
    /// The backend does not implement the requested operation.
    #[error("operation is not supported by this backend: {0}")]
    Unsupported(String),
    /// Encoding or decoding a typed value failed.
    #[error("serialization failed: {0}")]
    Serialization(String),
    /// The watched scope is not known to the pool.
    #[error("scope `{0}` is not known to this pool")]
    ScopeNotFound(String),
    /// A background task could not be joined.
    #[error("background task failed: {0}")]
    Task(String),
    /// The watch stream ended before another event arrived.
    #[error("the watch stream has ended")]
    WatchClosed,
}

impl SdkError {
    /// Return `true` when retrying the operation can plausibly succeed.
    ///
    /// OCC write conflicts and transient transport failures are retryable.
    /// Application and validation errors are not.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            SdkError::Core(StorkvError::WriteConflict { .. })
                | SdkError::Client(ClientError::Server(StorkvWireError::WriteConflict { .. }))
                | SdkError::Client(
                    ClientError::Io(_)
                        | ClientError::Http(_)
                        | ClientError::Grpc(_)
                        | ClientError::GrpcStatus(_)
                )
        )
    }

    /// Return `true` when the failure is an OCC write conflict.
    #[must_use]
    pub fn is_write_conflict(&self) -> bool {
        matches!(
            self,
            SdkError::Core(StorkvError::WriteConflict { .. })
                | SdkError::Client(ClientError::Server(StorkvWireError::WriteConflict { .. }))
        )
    }
}

pub(crate) fn join_error(error: tokio::task::JoinError) -> SdkError {
    SdkError::Task(error.to_string())
}
