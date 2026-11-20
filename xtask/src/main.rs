//! Build and release automation for the StorKV workspace.

#![warn(missing_docs)]

mod archive;
mod build_rust;
mod cli;
mod error;
mod metadata;
mod sandbox;
mod stage_native;
mod stage_sdks;
mod test_runner;
mod tools;

use std::process::ExitCode;

use cli::{Action, DistOptions, TestOptions};
use error::Result;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let action = cli::parse(std::env::args().skip(1))?;
    let root = metadata::workspace_root()?;
    match action {
        Action::Help => {
            print!("{}", cli::usage());
            Ok(())
        }
        Action::VerifyHeaders => build_rust::verify_header(&root),
        Action::Dist(options) => run_dist(&root, options),
        Action::Test(options) => run_test(&root, options),
        Action::Sandbox(options) => sandbox::run(&root, &options),
    }
}

fn run_dist(root: &std::path::Path, options: DistOptions) -> Result<()> {
    let version = metadata::workspace_version(root)?;
    let target = metadata::target_triple(options.target.as_deref())?;
    build_rust::build_workspace(root, &target)?;
    build_rust::generate_header(root)?;
    let stage = stage_native::stage(root, &version, &target)?;
    stage_sdks::stage(root, &version, &target, &stage, options.strict_sdks)?;
    if !options.skip_archive {
        archive::create(root, &version, &target, &stage)?;
    }
    println!("staged {}", stage.display());
    Ok(())
}

fn run_test(root: &std::path::Path, options: TestOptions) -> Result<()> {
    test_runner::run(
        root,
        options.tier,
        options.live_uri.as_deref(),
        options.with_server,
        options.proptest_cases,
    )
}
