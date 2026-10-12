mod build_support;
use std::{env, fs, path::PathBuf};
fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../..")
        .canonicalize()
        .unwrap();
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    println!("cargo:rerun-if-env-changed=TOUCHBAR_WASM_FIXTURES");
    for path in [
        "plugins",
        "tests/logic-fixtures",
        "crates/touchbar-logic-sdk",
        "crates/touchbar-logic-types",
        "crates/touchbar-model",
        "wit",
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "crates/touchbar-wasm-fixtures/build_support.rs",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    let paths = if let Some(dir) = env::var_os("TOUCHBAR_WASM_FIXTURES") {
        let dir = PathBuf::from(dir)
            .canonicalize()
            .expect("fixture override directory must exist");
        build_support::MODULES
            .iter()
            .map(|(name, _)| {
                let path = dir.join(format!("{}.wasm", name.to_ascii_lowercase()));
                assert!(
                    path.is_file(),
                    "missing prebuilt fixture: {}",
                    path.display()
                );
                (*name, path)
            })
            .collect()
    } else {
        build_support::build(&root, &output)
            .unwrap_or_else(|error| panic!("build wasm components:\n{error}"))
    };
    let source: String = paths
        .iter()
        .map(|(name, path)| {
            format!(
                "pub const {name}: &str = {:?};\n",
                path.to_str().expect("UTF-8 fixture path")
            )
        })
        .collect();
    fs::write(output.join("fixtures.rs"), source).unwrap();
}
