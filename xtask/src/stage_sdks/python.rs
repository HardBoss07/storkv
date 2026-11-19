//! Build the Python wheel and source distribution.

use std::path::Path;

use crate::error::{Result, XtaskError};
use crate::tools;

use super::{release_dir, shared_libraries};

/// Build Python distribution artifacts with the native library inside.
///
/// # Errors
///
/// Returns an error when Python is missing or a build command fails.
pub(super) fn stage(root: &Path, target: &str, sdk: &Path) -> Result<()> {
    let python = interpreter()?;
    let work = root.join("dist").join("tmp").join("python");
    tools::reset_dir(&work)?;
    tools::copy_tree(&root.join("bindings").join("python"), &work)?;
    let package = work.join("storkv");
    for library in shared_libraries(&release_dir(root, target))? {
        let name = library
            .file_name()
            .ok_or_else(|| XtaskError::Layout("native library has no file name".into()))?;
        tools::copy_file(&library, &package.join(name))?;
    }
    let output = sdk.join("python");
    let output_argument = output.to_string_lossy().into_owned();
    let result = build(
        &python,
        &work,
        &["-m", "build", "--outdir", &output_argument],
    );
    if result.is_err() {
        build(
            &python,
            &work,
            &[
                "-m",
                "pip",
                "wheel",
                "--no-deps",
                "--wheel-dir",
                &output_argument,
                ".",
            ],
        )?;
    }
    Ok(())
}

fn interpreter() -> Result<String> {
    for candidate in ["python", "python3"] {
        if tools::available(candidate) {
            return Ok(candidate.to_string());
        }
    }
    Err(XtaskError::MissingTool("python".into()))
}

fn build(python: &str, directory: &Path, arguments: &[&str]) -> Result<()> {
    let mut command = tools::tool(python);
    command.current_dir(directory).args(arguments);
    tools::run(command, "python build")?;
    Ok(())
}
