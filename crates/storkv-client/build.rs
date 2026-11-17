use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc_path = protoc_bin_vendored::protoc_bin_path()?;
    let protoc_include = protoc_bin_vendored::include_path()?;

    // SAFETY: build scripts run single-threaded before any other thread is spawned.
    unsafe {
        std::env::set_var("PROTOC", &protoc_path);
    }

    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let proto_root = workspace_root.join("proto");
    let proto_file = proto_root.join("storkv/v1/storkv.proto");

    println!("cargo:rerun-if-changed={}", proto_file.display());

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(false)
        .compile_protos(&[&proto_file], &[&proto_root, &protoc_include])?;

    Ok(())
}
