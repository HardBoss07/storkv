//! Stage binaries, headers, libraries, and metadata for one target.

use std::path::{Path, PathBuf};

use crate::error::{Result, XtaskError};
use crate::metadata;
use crate::tools;

const BINARIES: [&str; 3] = ["storkvd", "storkvctl", "storkv-top"];
const LIBRARY_EXTENSIONS: [&str; 5] = ["dll", "so", "dylib", "a", "lib"];

/// Return the bundle directory name for one version and target.
pub fn bundle_name(version: &str, target: &str) -> String {
    format!("storkv-{version}-{target}")
}

/// Create the native bundle under `dist/`.
///
/// # Errors
///
/// Returns an error when an input file is missing or a copy fails.
pub fn stage(root: &Path, version: &str, target: &str) -> Result<PathBuf> {
    let stage = root.join("dist").join(bundle_name(version, target));
    tools::reset_dir(&stage)?;
    let release = metadata::release_dir(root, target);
    copy_binaries(&release, &stage)?;
    copy_includes(root, &stage)?;
    let libraries = copy_libraries(&release, &stage)?;
    write_pkg_config(&stage, version)?;
    let shared = shared_library_name(&libraries)?;
    write_cmake(&stage, version, target, &shared)?;
    copy_docs(root, &stage)?;
    Ok(stage)
}

fn copy_binaries(release: &Path, stage: &Path) -> Result<()> {
    let suffix = std::env::consts::EXE_SUFFIX;
    for binary in BINARIES {
        let name = format!("{binary}{suffix}");
        let source = release.join(&name);
        if !source.is_file() {
            return Err(XtaskError::Layout(format!(
                "missing binary {}",
                source.display()
            )));
        }
        tools::copy_file(&source, &stage.join("bin").join(name))?;
    }
    Ok(())
}

fn copy_includes(root: &Path, stage: &Path) -> Result<()> {
    let include = stage.join("include");
    tools::copy_file(
        &root.join("bindings").join("c").join("storkv.h"),
        &include.join("storkv.h"),
    )?;
    tools::copy_file(
        &root.join("bindings").join("cpp").join("storkv.hpp"),
        &include.join("storkv.hpp"),
    )
}

fn copy_libraries(release: &Path, stage: &Path) -> Result<Vec<PathBuf>> {
    let mut copied = Vec::new();
    for entry in std::fs::read_dir(release)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let extension = path.extension().and_then(|value| value.to_str());
        let is_ffi = name.starts_with("storkv_ffi");
        let known = extension.is_some_and(|value| LIBRARY_EXTENSIONS.contains(&value));
        if is_ffi && known {
            tools::copy_file(&path, &stage.join("lib").join(&name))?;
            copied.push(PathBuf::from(name));
        }
    }
    if copied.is_empty() {
        return Err(XtaskError::Layout(format!(
            "no storkv_ffi library in {}",
            release.display()
        )));
    }
    Ok(copied)
}

fn shared_library_name(libraries: &[PathBuf]) -> Result<String> {
    libraries
        .iter()
        .find(|path| {
            let name = path.to_string_lossy();
            name.ends_with(".dll") || name.ends_with(".so") || name.ends_with(".dylib")
        })
        .map(|path| path.to_string_lossy().into_owned())
        .ok_or_else(|| XtaskError::Layout("no shared storkv_ffi library".into()))
}

fn write_pkg_config(stage: &Path, version: &str) -> Result<()> {
    let content = format!(
        "prefix=${{pcfiledir}}/../..\n\
         libdir=${{prefix}}/lib\n\
         includedir=${{prefix}}/include\n\n\
         Name: storkv\n\
         Description: StorKV storage engine C ABI\n\
         Version: {version}\n\
         Libs: -L${{libdir}} -lstorkv_ffi\n\
         Cflags: -I${{includedir}}\n"
    );
    let path = stage.join("lib").join("pkgconfig").join("storkv.pc");
    write_text(&path, &content)
}

fn write_cmake(stage: &Path, version: &str, target: &str, shared: &str) -> Result<()> {
    let implib = if target.contains("windows") {
        String::from(
            "set_target_properties(Storkv::storkv_ffi PROPERTIES IMPORTED_IMPLIB \"${_storkv_prefix}/lib/storkv_ffi.dll.lib\")",
        )
    } else {
        String::new()
    };
    let content = format!(
        "set(Storkv_VERSION \"{version}\")\n\
         get_filename_component(_storkv_prefix \"${{CMAKE_CURRENT_LIST_DIR}}/../../..\" ABSOLUTE)\n\
         add_library(Storkv::storkv_ffi SHARED IMPORTED)\n\
         set_target_properties(Storkv::storkv_ffi PROPERTIES\n\
         \x20 IMPORTED_LOCATION \"${{_storkv_prefix}}/lib/{shared}\"\n\
         \x20 INTERFACE_INCLUDE_DIRECTORIES \"${{_storkv_prefix}}/include\")\n\
         {implib}\n"
    );
    let path = stage
        .join("lib")
        .join("cmake")
        .join("Storkv")
        .join("StorkvConfig.cmake");
    write_text(&path, &content)
}

fn copy_docs(root: &Path, stage: &Path) -> Result<()> {
    for name in ["SPEC.md", "FORMAT.md", "README.md", "LICENSE"] {
        tools::copy_file(&root.join(name), &stage.join("docs").join(name))?;
    }
    Ok(())
}

fn write_text(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}
