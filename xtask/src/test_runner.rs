//! Test tiers that the `test` command can run.

use std::path::Path;
use std::process::Command;

use crate::build_rust;
use crate::error::Result;
use crate::tools;

mod bindings;

/// Test tier selected on the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Quick checks: format, lint, and the workspace tests.
    Fast,
    /// Pull request checks: `fast` plus the generated header and docs.
    Pr,
    /// Full checks: `pr` plus a release build.
    Full,
    /// Language binding tests.
    Bindings,
}

/// Run one test tier.
///
/// # Errors
///
/// Returns an error when a check fails or a tool is missing.
pub fn run(root: &Path, tier: Tier, live_uri: Option<&str>, with_server: bool) -> Result<()> {
    match tier {
        Tier::Fast => fast(root),
        Tier::Pr => pr(root),
        Tier::Full => full(root),
        Tier::Bindings => bindings::run(root, live_uri, with_server),
    }
}

fn fast(root: &Path) -> Result<()> {
    cargo(
        root,
        &["fmt", "--all", "--", "--check"],
        "cargo fmt --check",
    )?;
    cargo(
        root,
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        "cargo clippy",
    )?;
    cargo(root, &["test", "--workspace"], "cargo test --workspace")
}

fn pr(root: &Path) -> Result<()> {
    fast(root)?;
    build_rust::verify_header(root)?;
    cargo(
        root,
        &["doc", "--workspace", "--no-deps"],
        "cargo doc --workspace",
    )
}

fn full(root: &Path) -> Result<()> {
    pr(root)?;
    cargo(
        root,
        &["build", "--workspace", "--release"],
        "cargo build --workspace --release",
    )
}

/// Run one cargo command in the workspace root.
///
/// # Errors
///
/// Returns an error when cargo cannot start or exits with failure.
pub(crate) fn cargo(root: &Path, arguments: &[&str], label: &str) -> Result<()> {
    let mut command = Command::new("cargo");
    command.current_dir(root).args(arguments);
    tools::run(command, label)?;
    Ok(())
}
