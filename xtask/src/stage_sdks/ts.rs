//! Build the TypeScript npm package.

use std::path::Path;

use crate::error::{Result, XtaskError};
use crate::tools;

/// Build and pack the TypeScript binding.
///
/// # Errors
///
/// Returns an error when npm is missing or a command fails.
pub(super) fn stage(root: &Path, sdk: &Path) -> Result<()> {
    if !tools::available("npm") {
        return Err(XtaskError::MissingTool("npm".into()));
    }
    let binding = root.join("bindings").join("ts");
    let install = if binding.join("package-lock.json").is_file() {
        "ci"
    } else {
        "install"
    };
    run(&binding, &["npm", install], "npm install")?;
    run(&binding, &["npm", "run", "build"], "npm run build")?;
    let output = run(&binding, &["npm", "pack"], "npm pack")?;
    let archive = String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.ends_with(".tgz"))
        .ok_or_else(|| XtaskError::Layout("npm pack printed no tarball name".into()))?
        .to_string();
    let destination = sdk.join("ts");
    std::fs::create_dir_all(&destination)?;
    std::fs::rename(binding.join(&archive), destination.join(&archive))?;
    Ok(())
}

fn run(directory: &Path, command: &[&str], label: &str) -> Result<std::process::Output> {
    let mut process = tools::tool(command[0]);
    process.current_dir(directory).args(&command[1..]);
    tools::run(process, label)
}
