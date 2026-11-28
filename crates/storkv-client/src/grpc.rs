//! gRPC transport types and request conversion.
#[allow(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    unused_qualifications,
    missing_docs
)]
mod inner {
    include!(concat!(env!("OUT_DIR"), "/storkv.v1.rs"));
}

pub use inner::*;
