//! Process helpers with clear error messages.

use std::path::Path;
use std::process::{Command, Output};

use crate::error::{Result, XtaskError};

/// Run one command and return its captured output.
///
/// # Errors
///
/// Returns an error when the command cannot start or exits with failure.
pub fn run(mut command: Command, label: &str) -> Result<Output> {
    eprintln!("+ {label}");
    let output = command
        .output()
        .map_err(|error| XtaskError::MissingTool(format!("{label} ({error})")))?;
    if output.status.success() {
        return Ok(output);
    }
    Err(XtaskError::Command {
        program: label.to_string(),
        status: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Create a command for an external tool. Uses `cmd /C` on Windows.
pub fn tool(program: &str) -> Command {
    if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.arg("/C").arg(program);
        command
    } else {
        Command::new(program)
    }
}

/// Return `true` when a tool responds to `--version`.
pub fn available(program: &str) -> bool {
    tool(program)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Return `true` when a program file exists on `PATH`.
///
/// Use this for tools that reject `--version`, such as the MSVC compiler.
pub fn on_path(program: &str) -> bool {
    let extensions: &[&str] = if cfg!(windows) {
        &["", ".exe", ".bat", ".cmd"]
    } else {
        &[""]
    };
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| {
        extensions
            .iter()
            .any(|extension| directory.join(format!("{program}{extension}")).is_file())
    })
}

/// Copy one file and create the parent directory.
///
/// # Errors
///
/// Returns an error when the copy fails.
pub fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source, destination)?;
    Ok(())
}

/// Recursively copy a directory tree.
///
/// # Errors
///
/// Returns an error when the tree cannot be read or copied.
pub fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Remove a directory when it exists.
///
/// # Errors
///
/// Returns an error when removal fails.
pub fn reset_dir(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)?;
    }
    std::fs::create_dir_all(path)?;
    Ok(())
}
