//! Error type for the release tasks.

use std::fmt;

/// Failure returned by an xtask command.
#[derive(Debug)]
pub enum XtaskError {
    /// A file system operation failed.
    Io(std::io::Error),
    /// An external command exited with a non-zero status.
    Command {
        /// Command line label that failed.
        program: String,
        /// Process exit code, when available.
        status: Option<i32>,
        /// Captured standard error text.
        stderr: String,
    },
    /// A required external tool is not on the PATH.
    MissingTool(String),
    /// A command line argument is invalid.
    InvalidArgument(String),
    /// The workspace layout does not match the expected layout.
    Layout(String),
    /// The generated C header is stale.
    StaleHeader(String),
}

impl fmt::Display for XtaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Command {
                program,
                status,
                stderr,
            } => write!(
                formatter,
                "command failed: {program} (status {status:?}): {}",
                stderr.trim()
            ),
            Self::MissingTool(tool) => write!(formatter, "required tool is missing: {tool}"),
            Self::InvalidArgument(message) => write!(formatter, "invalid argument: {message}"),
            Self::Layout(message) => write!(formatter, "layout error: {message}"),
            Self::StaleHeader(message) => write!(formatter, "stale header: {message}"),
        }
    }
}

impl std::error::Error for XtaskError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for XtaskError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Result type used by every xtask command.
pub type Result<T> = std::result::Result<T, XtaskError>;
