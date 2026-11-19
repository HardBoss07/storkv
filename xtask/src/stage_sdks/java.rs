//! Build the Java JAR with the native library inside.

use std::path::Path;

use crate::error::{Result, XtaskError};
use crate::tools;

use super::{release_dir, shared_libraries};

/// Build the Java binding JAR.
///
/// # Errors
///
/// Returns an error when a Java tool is missing or a command fails.
pub(super) fn stage(root: &Path, version: &str, target: &str, sdk: &Path) -> Result<()> {
    if !tools::available("javac") || !tools::available("jar") {
        return Err(XtaskError::MissingTool("javac and jar".into()));
    }
    let classes = root.join("dist").join("tmp").join("java").join("classes");
    tools::reset_dir(&classes)?;
    let classes_argument = classes.to_string_lossy().into_owned();
    compile(root, &classes_argument)?;
    let native = classes
        .join("META-INF")
        .join("native")
        .join(platform_directory(target));
    let release = release_dir(root, target);
    for library in shared_libraries(&release)? {
        let name = library
            .file_name()
            .ok_or_else(|| XtaskError::Layout("native library has no file name".into()))?;
        tools::copy_file(&library, &native.join(name))?;
    }
    let output = sdk.join("java");
    std::fs::create_dir_all(&output)?;
    let jar = output.join(format!("storkv-client-{version}.jar"));
    let jar_argument = jar.to_string_lossy().into_owned();
    let mut package = tools::tool("jar");
    package.args([
        "--create",
        "--file",
        &jar_argument,
        "-C",
        &classes_argument,
        ".",
    ]);
    tools::run(package, "jar")?;
    Ok(())
}

fn compile(root: &Path, classes: &str) -> Result<()> {
    let source = "bindings/java/storkv/StorkvClient.java";
    let mut release = tools::tool("javac");
    release
        .current_dir(root)
        .args(["--release", "22", "-d", classes, source]);
    if tools::run(release, "javac --release 22").is_ok() {
        return Ok(());
    }
    let mut preview = tools::tool("javac");
    preview
        .current_dir(root)
        .args(["--release", "21", "--enable-preview", "-d", classes, source]);
    tools::run(preview, "javac --release 21 --enable-preview")?;
    Ok(())
}

fn platform_directory(target: &str) -> String {
    let os = if target.contains("windows") {
        "windows"
    } else if target.contains("darwin") {
        "macos"
    } else {
        "linux"
    };
    let arch = if target.starts_with("aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    format!("{os}-{arch}")
}
