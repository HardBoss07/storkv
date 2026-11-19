use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let config_path = manifest.join("cbindgen.toml");
    let config = cbindgen::Config::from_file(&config_path).expect("read cbindgen config");
    let bindings = cbindgen::Builder::new()
        .with_crate(&manifest)
        .with_config(config)
        .generate()
        .expect("generate header");
    let output = manifest
        .join("..")
        .join("..")
        .join("bindings")
        .join("c")
        .join("storkv.h");
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).expect("create bindings directory");
    }
    bindings.write_to_file(&output);
    println!("wrote {}", output.display());
}
