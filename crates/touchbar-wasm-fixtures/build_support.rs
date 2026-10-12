//! Shared component build support; reusable by the future xtask.
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const MODULES: &[(&str, &str)] = &[
    ("CONTROLS", "touchbar-controls"),
    ("ECHO", "touchbar-fixture-echo"),
    ("TRAP", "touchbar-fixture-trap"),
    ("HANG", "touchbar-fixture-hang"),
    ("OOM", "touchbar-fixture-oom"),
    ("ERR", "touchbar-fixture-err"),
    ("BUDGET", "touchbar-fixture-budget"),
    ("LOG", "touchbar-fixture-log"),
    ("PERSIST", "touchbar-fixture-persist"),
    ("VERSION", "touchbar-fixture-version"),
    ("WASI", "touchbar-fixture-wasi"),
];

pub fn build(root: &Path, output: &Path) -> Result<Vec<(&'static str, PathBuf)>, String> {
    let target = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"))
        .join("logic-fixtures");
    let mut cargo = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo
        .current_dir(root)
        .args([
            "build",
            "--release",
            "--locked",
            "--target",
            "wasm32-unknown-unknown",
            "--target-dir",
        ])
        .arg(&target);
    for (_, package) in MODULES {
        cargo.args(["-p", package]);
    }
    for (name, _) in env::vars_os() {
        let key = name.to_string_lossy();
        if matches!(
            key.as_ref(),
            "RUSTFLAGS"
                | "CARGO_ENCODED_RUSTFLAGS"
                | "RUSTDOCFLAGS"
                | "CARGO_TARGET_DIR"
                | "CARGO_MAKEFLAGS"
                | "MAKEFLAGS"
        ) || key.starts_with("CARGO_CFG_")
            || key.starts_with("CARGO_FEATURE_")
        {
            cargo.env_remove(name);
        }
    }
    let result = cargo
        .output()
        .map_err(|e| format!("launch wasm cargo: {e}"))?;
    if !result.status.success() {
        return Err(format!(
            "wasm cargo failed: {}\n{}",
            result.status,
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    let raw_dir = target.join("wasm32-unknown-unknown/release");
    MODULES
        .iter()
        .map(|(name, package)| {
            let raw = fs::read(raw_dir.join(format!("{}.wasm", package.replace('-', "_"))))
                .map_err(|e| format!("read {package}: {e}"))?;
            let component = wit_component::ComponentEncoder::default()
                .module(&raw)
                .map_err(|e| format!("metadata {package}: {e:#}"))?
                .validate(true)
                .encode()
                .map_err(|e| format!("encode {package}: {e:#}"))?;
            let path = output.join(format!("{}.wasm", name.to_ascii_lowercase()));
            fs::write(&path, component).map_err(|e| format!("write {package}: {e}"))?;
            Ok((*name, path))
        })
        .collect()
}
