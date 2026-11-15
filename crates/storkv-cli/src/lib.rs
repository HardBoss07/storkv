//! `storkvctl` command bodies: health checks, backups, offline doctor
//! verification, and YCSB-style workload generation.

mod backup;
mod doctor;
mod health;
mod ycsb;

pub use doctor::DoctorReport;
pub use ycsb::YcsbSummary;

pub struct CliRunner;
