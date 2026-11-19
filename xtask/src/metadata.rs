//! Workspace metadata: version, paths, and target triple.

use std::path::{Path, PathBuf};

use crate::error::{Result, XtaskError};

/// Return the workspace root directory.
///
/// # Errors
///
/// Returns an error when the xtask manifest directory has no parent.
pub fn workspace_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| XtaskError::Layout("xtask directory has no parent".into()))
}

/// Read the workspace version from the root manifest.
///
/// # Errors
///
/// Returns an error when the manifest is unreadable or has no version.
pub fn workspace_version(root: &Path) -> Result<String> {
    let manifest = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)?;
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[workspace.package]";
            continue;
        }
        if in_package
            && let Some(value) = trimmed.strip_prefix("version")
            && let Some(value) = value.trim().strip_prefix('=')
        {
            return Ok(value.trim().trim_matches('"').to_string());
        }
    }
    Err(XtaskError::Layout(format!(
        "no workspace.package.version in {}",
        manifest.display()
    )))
}

/// Return the requested target triple or the host triple.
///
/// # Errors
///
/// Returns an error when `rustc` does not report a host triple.
pub fn target_triple(requested: Option<&str>) -> Result<String> {
    if let Some(target) = requested {
        return Ok(target.to_string());
    }
    let output = std::process::Command::new("rustc")
        .arg("-vV")
        .output()
        .map_err(|error| XtaskError::MissingTool(format!("rustc ({error})")))?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if let Some(host) = line.strip_prefix("host: ") {
            return Ok(host.trim().to_string());
        }
    }
    Err(XtaskError::Layout(
        "rustc did not report a host triple".into(),
    ))
}

/// Return the release directory for one target.
pub fn release_dir(root: &Path, target: &str) -> PathBuf {
    root.join("target").join(target).join("release")
}
