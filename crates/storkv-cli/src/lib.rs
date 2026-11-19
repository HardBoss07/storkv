//! `storkvctl` command bodies: health checks, backups, offline doctor
//! verification, migration, restore, and YCSB-style workload generation.
//!
//! The `storkvctl` binary is a thin argument parser. This library exposes the
//! reusable pieces so that tests and other tools can call the same operations.

#![warn(missing_docs)]

mod backup;
mod doctor;
mod health;
mod migrate;
mod restore;
mod ycsb;

pub use doctor::DoctorReport;
pub use migrate::MigrateTarget;
pub use ycsb::YcsbSummary;

/// Entry point used by the `storkvctl` binary after argument parsing.
pub struct CliRunner;
