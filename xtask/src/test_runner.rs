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
    /// Every check and every language binding suite in one pass.
    All,
}

/// Run one test tier.
///
/// # Errors
///
/// Returns an error when a check fails or a tool is missing.
pub fn run(
    root: &Path,
    tier: Tier,
    live_uri: Option<&str>,
    with_server: bool,
    proptest_cases: u32,
) -> Result<()> {
    match tier {
        Tier::Fast => fast(root),
        Tier::Pr => pr(root),
        Tier::Full => full(root),
        Tier::Bindings => bindings::run(root, live_uri, with_server),
        Tier::All => all(root, live_uri, with_server, proptest_cases),
    }
}

fn fast(root: &Path) -> Result<()> {
    checks(root)?;
    cargo(root, &["test", "--workspace"], "cargo test --workspace")
}

fn pr(root: &Path) -> Result<()> {
    checks(root)?;
    // Run every workspace integration suite on pull requests, not just the
    // fast unit subset: proptest_oracle, tls_auth_matrix, cli, ffi, and the
    // compatibility tests all live under `tests/`. Long-running chaos tests
    // are reserved for the `full` tier.
    cargo(
        root,
        &["test", "--workspace", "--", "--skip", "chaos"],
        "cargo test --workspace -- --skip chaos",
    )?;
    build_rust::verify_header(root)?;
    cargo(
        root,
        &["doc", "--workspace", "--no-deps"],
        "cargo doc --workspace",
    )
}

fn checks(root: &Path) -> Result<()> {
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

fn all(root: &Path, live_uri: Option<&str>, with_server: bool, proptest_cases: u32) -> Result<()> {
    checks(root)?;
    build_rust::verify_header(root)?;
    cargo(
        root,
        &["doc", "--workspace", "--no-deps"],
        "cargo doc --workspace",
    )?;

    let proptest = [("PROPTEST_CASES", proptest_cases.to_string())];
    cargo_with_env(
        root,
        &["test", "--workspace"],
        "cargo test --workspace",
        &proptest,
    )?;

    if live_uri.is_some() || with_server {
        let mut environment = vec![("PROPTEST_CASES", proptest_cases.to_string())];
        if let Some(uri) = live_uri {
            environment.push(("STORKV_TEST_URI", uri.to_string()));
        }
        cargo_with_env(
            root,
            &["test", "--workspace", "--", "--ignored"],
            "cargo test --workspace -- --ignored",
            &environment,
        )?;
    }

    bindings::run(root, live_uri, with_server)
}

/// Run one cargo command in the workspace root.
///
/// # Errors
///
/// Returns an error when cargo cannot start or exits with failure.
pub(crate) fn cargo(root: &Path, arguments: &[&str], label: &str) -> Result<()> {
    cargo_with_env(root, arguments, label, &[])
}

fn cargo_with_env(
    root: &Path,
    arguments: &[&str],
    label: &str,
    environment: &[(&str, String)],
) -> Result<()> {
    let mut command = Command::new("cargo");
    command.current_dir(root).args(arguments);
    for (name, value) in environment {
        command.env(name, value);
    }
    tools::run(command, label)?;
    Ok(())
}
