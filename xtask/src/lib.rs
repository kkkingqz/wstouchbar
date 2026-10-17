//! Project build commands. Runtime plugin trees are published as a whole.
#[path = "../../crates/touchbar-wasm-fixtures/build_support.rs"]
pub mod build_support;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};
const MARKER: &str = "wstouchbar-build-plugins-v1\n";
fn absolute(path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.into()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            _ => normal.push(component.as_os_str()),
        }
    }
    let mut ancestor = normal.as_path();
    let mut missing = Vec::new();
    while !ancestor.exists() {
        missing.push(
            ancestor
                .file_name()
                .ok_or("invalid output path")?
                .to_owned(),
        );
        ancestor = ancestor.parent().ok_or("invalid output parent")?;
    }
    let mut resolved = ancestor.canonicalize().map_err(|e| e.to_string())?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}
fn destination(root: &Path, output: &Path) -> Result<PathBuf, String> {
    if fs::symlink_metadata(output).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("output must not be a symlink".into());
    }
    let source = root
        .join("plugins")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let output = absolute(output)?;
    if source.starts_with(&output) || output.starts_with(&source) {
        return Err("output must be outside source plugins and their ancestors".into());
    }
    if output.exists()
        && (!output.is_dir()
            || fs::read_to_string(output.join(".touchbar-built-plugins"))
                .ok()
                .as_deref()
                != Some(MARKER))
    {
        return Err(format!(
            "refusing to replace unowned output {}",
            output.display()
        ));
    }
    Ok(output)
}
fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let target = target.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target).map_err(|e| e.to_string())?;
        } else {
            return Err(format!(
                "plugin tree contains a symlink or special file: {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}
pub fn assemble_plugins(root: &Path, output: &Path, controls: &Path) -> Result<(), String> {
    let output = destination(root, output)?;
    let parent = output.parent().ok_or("output needs a parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let transaction = tempfile::Builder::new()
        .prefix(".touchbar-plugins-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    let tree = transaction.path().join("tree");
    copy_tree(&root.join("plugins"), &tree)?;
    fs::copy(controls, tree.join("controls/logic.wasm")).map_err(|e| e.to_string())?;
    fs::write(tree.join(".touchbar-built-plugins"), MARKER).map_err(|e| e.to_string())?;
    let previous = transaction.path().join("previous");
    let replacing = output.exists();
    if replacing {
        fs::rename(&output, &previous).map_err(|e| e.to_string())?;
    }
    if let Err(error) = fs::rename(&tree, &output) {
        if replacing && let Err(restore) = fs::rename(&previous, &output) {
            let saved = transaction.keep();
            return Err(format!(
                "publish failed: {error}; restore failed: {restore}; old output preserved in {}",
                saved.display()
            ));
        }
        return Err(format!("publish plugins: {error}"));
    }
    Ok(())
}
pub fn build_plugins(root: &Path, output: &Path) -> Result<(), String> {
    // Refuse unsafe destinations before compiling or creating output directories.
    destination(root, output)?;
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let modules = build_support::build_selected(
        root,
        temporary.path(),
        &[("CONTROLS", "touchbar-controls")],
    )?;
    assemble_plugins(root, output, &modules[0].1)
}

/// Hardware preflight is read-only; tests inject a sysfs tree and effective UID.
pub fn check_hardware(usb_devices: &Path, effective_uid: u32) -> Result<(), String> {
    if effective_uid != 0 {
        return Err("hardware environment requires root".into());
    }
    let entries = fs::read_dir(usb_devices)
        .map_err(|e| format!("hardware environment requires Touch Bar USB 05ac:8302: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot inspect USB devices: {e}"))?;
        let vendor = fs::read_to_string(entry.path().join("idVendor"));
        let product = fs::read_to_string(entry.path().join("idProduct"));
        if vendor.as_deref().is_ok_and(|v| v.trim() == "05ac")
            && product.as_deref().is_ok_and(|v| v.trim() == "8302")
        {
            return Ok(());
        }
    }
    Err("hardware environment requires Touch Bar USB 05ac:8302".into())
}
