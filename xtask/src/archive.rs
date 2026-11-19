//! Archive creation and checksum generation.

use std::path::Path;
use std::process::Command;

use crate::error::{Result, XtaskError};
use crate::stage_native::bundle_name;

/// Archive the staged bundle and write its SHA-256 checksum.
///
/// # Errors
///
/// Returns an error when archiving or hashing fails.
pub fn create(root: &Path, version: &str, target: &str, stage: &Path) -> Result<()> {
    if !stage.is_dir() {
        return Err(XtaskError::Layout(format!(
            "staged bundle is missing: {}",
            stage.display()
        )));
    }
    let dist = root.join("dist");
    let name = bundle_name(version, target);
    let archive = if cfg!(windows) {
        dist.join(format!("{name}.zip"))
    } else {
        dist.join(format!("{name}.tar.gz"))
    };
    if cfg!(windows) {
        create_zip(&dist, &name, &archive)?;
    } else {
        tar(&dist, &name, &archive)?;
    }
    let digest = sha256(&archive)?;
    let checksums = dist.join(format!("SHA256SUMS-{target}.txt"));
    let file_name = archive
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .ok_or_else(|| XtaskError::Layout("archive has no file name".into()))?;
    std::fs::write(checksums, format!("{digest}  {file_name}\n"))?;
    Ok(())
}

fn tar(dist: &Path, name: &str, archive: &Path) -> Result<()> {
    let mut command = Command::new("tar");
    command
        .current_dir(dist)
        .args(["-czf"])
        .arg(archive)
        .arg(name);
    crate::tools::run(command, "tar -czf")?;
    Ok(())
}

fn create_zip(dist: &Path, name: &str, archive: &Path) -> Result<()> {
    let mut tar = Command::new("tar");
    tar.current_dir(dist)
        .arg("-a")
        .arg("-c")
        .arg("-f")
        .arg(archive)
        .arg(name);
    if crate::tools::run(tar, "tar -a -c -f").is_ok() {
        return Ok(());
    }
    let script = format!(
        "Compress-Archive -Path '{}' -DestinationPath '{}' -Force",
        dist.join(name).display(),
        archive.display()
    );
    let mut powershell = Command::new("powershell");
    powershell.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    crate::tools::run(powershell, "Compress-Archive")?;
    Ok(())
}

fn sha256(path: &Path) -> Result<String> {
    if cfg!(windows) {
        return certutil_hash(path);
    }
    for program in ["sha256sum", "shasum"] {
        if !crate::tools::available(program) {
            continue;
        }
        let mut command = Command::new(program);
        if program == "shasum" {
            command.args(["-a", "256"]);
        }
        command.arg(path);
        let output = crate::tools::run(command, program)?;
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some(hash) = text.split_whitespace().next() {
            return Ok(hash.to_ascii_lowercase());
        }
    }
    Err(XtaskError::MissingTool("sha256sum or shasum".into()))
}

fn certutil_hash(path: &Path) -> Result<String> {
    let mut command = Command::new("certutil");
    command.args(["-hashfile"]).arg(path).arg("SHA256");
    let output = crate::tools::run(command, "certutil -hashfile")?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let compact: String = line
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        if compact.len() == 64 && compact.chars().all(|value| value.is_ascii_hexdigit()) {
            return Ok(compact.to_ascii_lowercase());
        }
    }
    Err(XtaskError::Layout(
        "certutil printed no SHA-256 digest".into(),
    ))
}
