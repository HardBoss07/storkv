//! Virtual filesystem abstraction with a POSIX implementation, an in-memory
//! backend, a filesystem selector, and a fault-injecting wrapper for chaos
//! tests.

mod any;
mod fault_injecting;
mod memory;
mod posix;
mod traits;

pub use any::AnyFs;
pub use fault_injecting::{FaultInjectingFile, FaultInjectingFs};
pub use memory::{InMemoryFile, InMemoryFs};
pub use posix::{PosixFile, PosixFs};
pub use traits::{File, FileSystem};
