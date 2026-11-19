//! Package the TypeScript, Python, and Java SDKs.

use std::path::{Path, PathBuf};

use crate::error::{Result, XtaskError};
use crate::metadata;

mod java;
mod python;
mod ts;

/// Create the SDK artifacts inside one native bundle.
///
/// # Errors
///
/// Returns an error when a strict SDK build fails.
pub fn stage(root: &Path, version: &str, target: &str, stage: &Path, strict: bool) -> Result<()> {
    let sdk = stage.join("sdk");
    std::fs::create_dir_all(&sdk)?;
    attempt("TypeScript", strict, || ts::stage(root, &sdk))?;
    attempt("Python", strict, || python::stage(root, target, &sdk))?;
    attempt("Java", strict, || java::stage(root, version, target, &sdk))
}

fn attempt(name: &str, strict: bool, build: impl FnOnce() -> Result<()>) -> Result<()> {
    match build() {
        Ok(()) => Ok(()),
        Err(error) if strict => Err(XtaskError::Command {
            program: format!("{name} SDK"),
            status: None,
            stderr: error.to_string(),
        }),
        Err(error) => {
            eprintln!("warning: skipped {name} SDK: {error}");
            Ok(())
        }
    }
}

pub(super) fn release_dir(root: &Path, target: &str) -> PathBuf {
    metadata::release_dir(root, target)
}

pub(super) fn shared_libraries(release: &Path) -> Result<Vec<PathBuf>> {
    let mut libraries = Vec::new();
    for entry in std::fs::read_dir(release)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let shared = name.ends_with(".dll") || name.ends_with(".so") || name.ends_with(".dylib");
        if entry.file_type()?.is_file() && name.starts_with("storkv_ffi") && shared {
            libraries.push(entry.path());
        }
    }
    if libraries.is_empty() {
        return Err(XtaskError::Layout(format!(
            "no shared storkv_ffi library in {}",
            release.display()
        )));
    }
    Ok(libraries)
}
