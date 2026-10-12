//! Derive contract-breaking guests from the real v1 contract, without copied API types.
use std::{env, fs, path::PathBuf};
pub fn generate() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../..");
    let source = root.join("wit/logic.wit");
    println!("cargo:rerun-if-changed={}", source.display());
    println!(
        "cargo:rerun-if-changed={}",
        root.join("tests/logic-fixtures/contract_build.rs")
            .display()
    );
    let mut wit = fs::read_to_string(&source).unwrap();
    let wasi = env::var("CARGO_PKG_NAME").unwrap().ends_with("wasi");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let dir = out.join("wit");
    fs::create_dir_all(&dir).unwrap();
    if wasi {
        assert!(wit.contains("    export api;"));
        wit = wit.replace(
            "    export api;",
            "    import wasi:cli/environment@0.2.0;\n    export api;",
        );
        let deps = dir.join("deps/wasi-cli");
        fs::create_dir_all(&deps).unwrap();
        // The standard get-environment signature suffices to expose a forbidden WASI capability.
        fs::write(deps.join("environment.wit"), "package wasi:cli@0.2.0;\ninterface environment { get-environment: func() -> list<tuple<string, string>>; }\n").unwrap();
    } else {
        assert!(wit.contains("wstouchbar:logic@1.0.0"));
        wit = wit.replacen("wstouchbar:logic@1.0.0", "wstouchbar:logic@9.0.0", 1);
    }
    fs::write(dir.join("logic.wit"), wit).unwrap();
    fs::write(
        out.join("bindings.rs"),
        format!(
            "wit_bindgen::generate!({{ path: {:?}, world: \"logic\", pub_export_macro: true, generate_all, }});\n",
            dir.to_str().unwrap()
        ),
    )
    .unwrap();
}
