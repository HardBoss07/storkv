//! One test run per supported binding language.

use std::path::Path;

use super::Context;
use crate::error::Result;
use crate::tools;

/// Return the binary name with the platform extension.
pub(super) fn executable(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn first_available(candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .find(|name| tools::available(name))
        .map(|name| (*name).to_string())
}

fn first_on_path(candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .find(|name| tools::on_path(name))
        .map(|name| (*name).to_string())
}

fn skip(what: &str, tool: &str) {
    println!("skipping {what}: `{tool}` is not on the PATH");
}

fn import_library(context: &Context) -> String {
    let directory = context.root.join("target").join("debug");
    let name = if cfg!(windows) {
        "storkv_ffi.dll.lib"
    } else {
        "libstorkv_ffi.so"
    };
    directory.join(name).to_string_lossy().into_owned()
}

pub(super) fn run_c(context: &Context) -> Result<()> {
    let output = context.root.join("target").join(executable("c-ffi-test"));
    if cfg!(windows) {
        let Some(compiler) = first_on_path(&["cl", "clang-cl"]) else {
            skip("C binding test", "cl");
            return Ok(());
        };
        let mut command = context.command(&compiler);
        command.args(["/nologo", "/std:c11", "/Ibindings/c"]);
        command
            .arg("bindings/c/test_c_ffi.c")
            .arg(import_library(context));
        command.arg(format!("/Fe{}", output.display()));
        tools::run(command, "compile C binding test")?;
        return run_test_binary(context, &output, "run C binding test");
    }
    let Some(compiler) = first_available(&["cc", "gcc", "clang"]) else {
        skip("C binding test", "cc");
        return Ok(());
    };
    let mut command = context.command(&compiler);
    command.args([
        "-std=c99",
        "-I",
        "bindings/c",
        "bindings/c/test_c_ffi.c",
        "-L",
        "target/debug",
        "-lstorkv_ffi",
        "-o",
    ]);
    command.arg(&output);
    tools::run(command, "compile C binding test")?;
    run_test_binary(context, &output, "run C binding test")
}

pub(super) fn run_cpp(context: &Context) -> Result<()> {
    let output = context.root.join("target").join(executable("cpp-ffi-test"));
    if cfg!(windows) {
        // `clang-cl` can fail against a newer MSVC standard library, so only
        // accept the matching MSVC compiler here.
        let Some(compiler) = first_on_path(&["cl"]) else {
            skip("C++ binding test", "cl");
            return Ok(());
        };
        let mut command = context.command(&compiler);
        command.args([
            "/nologo",
            "/std:c++20",
            "/EHsc",
            "/Ibindings/c",
            "/Ibindings/cpp",
        ]);
        command
            .arg("bindings/cpp/test_cpp_ffi.cpp")
            .arg(import_library(context));
        command.arg(format!("/Fe{}", output.display()));
        tools::run(command, "compile C++ binding test")?;
        return run_test_binary(context, &output, "run C++ binding test");
    }
    let Some(compiler) = first_available(&["c++", "g++", "clang++"]) else {
        skip("C++ binding test", "c++");
        return Ok(());
    };
    let mut command = context.command(&compiler);
    command.args([
        "-std=c++20",
        "-I",
        "bindings/c",
        "-I",
        "bindings/cpp",
        "bindings/cpp/test_cpp_ffi.cpp",
        "-L",
        "target/debug",
        "-lstorkv_ffi",
        "-o",
    ]);
    command.arg(&output);
    tools::run(command, "compile C++ binding test")?;
    run_test_binary(context, &output, "run C++ binding test")
}

fn run_test_binary(context: &Context, binary: &Path, label: &str) -> Result<()> {
    let mut command = context.command(binary.to_string_lossy().as_ref());
    let library_dir = context.root.join("target").join("debug");
    for variable in ["PATH", "LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH"] {
        let existing = std::env::var_os(variable).unwrap_or_default();
        let mut paths = vec![library_dir.clone()];
        paths.extend(std::env::split_paths(&existing));
        if let Ok(joined) = std::env::join_paths(paths) {
            command.env(variable, joined);
        }
    }
    tools::run(command, label)?;
    Ok(())
}

pub(super) fn run_python(context: &Context) -> Result<()> {
    if !tools::available("python") {
        skip("Python binding test", "python");
        return Ok(());
    }
    let mut command = context.command("python");
    command.args([
        "-m",
        "unittest",
        "discover",
        "-s",
        "bindings/python/tests",
        "-v",
    ]);
    tools::run(command, "run Python binding test")?;
    Ok(())
}

pub(super) fn run_java(context: &Context) -> Result<()> {
    if !tools::available("javac") || !tools::available("java") {
        skip("Java binding test", "javac");
        return Ok(());
    }
    let out = "bindings/java/out";
    let mut compile = context.command("javac");
    compile.args([
        "-d",
        out,
        "bindings/java/storkv/StorkvClient.java",
        "bindings/java/StorkvFfiTest.java",
    ]);
    tools::run(compile, "compile Java binding test")?;

    let mut run = context.command("java");
    run.args([
        "--enable-native-access=ALL-UNNAMED",
        "-cp",
        out,
        "StorkvFfiTest",
    ]);
    tools::run(run, "run Java binding test")?;
    Ok(())
}

pub(super) fn run_typescript(context: &Context) -> Result<()> {
    if !tools::available("npm") {
        skip("TypeScript binding test", "npm");
        return Ok(());
    }
    let package = context.root.join("bindings").join("ts");
    if !package.join("node_modules").is_dir() {
        let mut install = tools::tool("npm");
        install
            .args(["install", "--no-audit", "--no-fund"])
            .current_dir(&package);
        tools::run(install, "install TypeScript binding dependencies")?;
    }
    let mut test = tools::tool("npm");
    test.arg("run").arg("test").current_dir(&package);
    test.env("STORKV_FFI_LIB", &context.library);
    if let Some(uri) = &context.uri {
        test.env("STORKV_TEST_URI", uri);
    }
    tools::run(test, "run TypeScript binding test")?;
    Ok(())
}
