//! Cargo build and header generation steps.

use std::path::Path;
use std::process::Command;

use crate::error::{Result, XtaskError};
use crate::tools;

/// Build every workspace crate in release mode for one target.
///
/// # Errors
///
/// Returns an error when cargo fails.
pub fn build_workspace(root: &Path, target: &str) -> Result<()> {
    let mut command = Command::new("cargo");
    command.current_dir(root).args([
        "build",
        "--workspace",
        "--exclude",
        "xtask",
        "--release",
        "--target",
        target,
    ]);
    tools::run(command, "cargo build --workspace --release")?;
    Ok(())
}

/// Regenerate the C header from the FFI crate.
///
/// # Errors
///
/// Returns an error when the generator fails.
pub fn generate_header(root: &Path) -> Result<()> {
    let mut command = Command::new("cargo");
    command.current_dir(root).args([
        "run",
        "-p",
        "storkv-ffi",
        "--release",
        "--example",
        "generate_header",
    ]);
    tools::run(command, "cargo run -p storkv-ffi --example generate_header")?;
    Ok(())
}

/// Confirm that the checked-in header matches the FFI definitions.
///
/// # Errors
///
/// Returns an error when the header changes during regeneration.
pub fn verify_header(root: &Path) -> Result<()> {
    let header = root.join("bindings").join("c").join("storkv.h");
    let original = std::fs::read(&header)?;
    generate_header(root)?;
    let regenerated = std::fs::read(&header)?;
    if original != regenerated {
        std::fs::write(&header, &original)?;
    }
    if normalize_eol(&original) == normalize_eol(&regenerated) {
        return Ok(());
    }
    Err(XtaskError::StaleHeader(format!(
        "{} differs from the generated header; run cargo xtask dist",
        header.display()
    )))
}

fn normalize_eol(bytes: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .copied()
        .filter(|&byte| byte != b'\r')
        .collect()
}
