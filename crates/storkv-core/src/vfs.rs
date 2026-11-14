//! Virtual filesystem abstraction with a POSIX implementation and a
//! fault-injecting wrapper for chaos tests.

mod fault_injecting;
mod posix;
mod traits;

pub use fault_injecting::{FaultInjectingFile, FaultInjectingFs};
pub use posix::{PosixFile, PosixFs};
pub use traits::{File, FileSystem};
