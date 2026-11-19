use std::path::PathBuf;

#[test]
fn committed_header_matches_cbindgen() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let config =
        cbindgen::Config::from_file(manifest.join("cbindgen.toml")).expect("read cbindgen config");
    let bindings = cbindgen::Builder::new()
        .with_crate(&manifest)
        .with_config(config)
        .generate()
        .expect("generate header");
    let mut generated = Vec::new();
    bindings.write(&mut generated);
    let generated = String::from_utf8(generated).expect("utf8 header");
    let header_path = manifest
        .join("..")
        .join("..")
        .join("bindings")
        .join("c")
        .join("storkv.h");
    let committed = std::fs::read_to_string(&header_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", header_path.display()));
    assert_eq!(
        committed, generated,
        "bindings/c/storkv.h is stale; run cargo run -p storkv-ffi --example generate_header"
    );
}
