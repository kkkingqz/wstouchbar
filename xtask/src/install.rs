//! Package staging and manifest-scoped removal. Never starts or enables services.
use rustix::fs::{AtFlags, FlockOperation, Mode, OFlags, ResolveFlags, open, openat2, unlinkat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read,
    os::{fd::AsFd, unix::fs::PermissionsExt},
    path::{Component, Path, PathBuf},
};
const MANIFEST: &str = ".touchbar-install-manifest.json";
const RESOLVE: ResolveFlags = ResolveFlags::BENEATH
    .union(ResolveFlags::NO_SYMLINKS)
    .union(ResolveFlags::NO_MAGICLINKS);
#[derive(Debug)]
pub struct Artifacts {
    pub daemon: PathBuf,
    pub session: PathBuf,
    pub helper: PathBuf,
    pub ctl: PathBuf,
    pub plugins: PathBuf,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: PathBuf,
    sha256: String,
    mode: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    prefix: PathBuf,
    files: Vec<Entry>,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn prefix(path: &Path) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("prefix is not UTF-8")?;
    if !path.is_absolute()
        || text.chars().any(char::is_control)
        || path.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err(
            "prefix must be an absolute path without traversal or control characters".into(),
        );
    }
    Ok(path.components().collect())
}
fn relative(path: &Path) -> bool {
    path.to_str().is_some_and(|s| {
        !s.is_empty() && s.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
    }) && !path.is_absolute()
}
fn package_path(prefix: &Path, path: &Path) -> bool {
    let Ok(base) = prefix.strip_prefix("/") else {
        return false;
    };
    let Ok(p) = path.strip_prefix(base) else {
        return false;
    };
    matches!(
        p.to_str(),
        Some(
            "bin/touchbarctl"
                | "lib/touchbar/touchbard"
                | "lib/touchbar/touchbar-session"
                | "lib/touchbar/touchbar-spawn-helper"
                | "lib/systemd/system/touchbard.service"
                | "lib/systemd/user/touchbar-session.service"
                | "lib/udev/rules.d/90-touchbar.rules"
                | "lib/udev/rules.d/99-wstouchbar-isolation.rules"
                | "lib/tmpfiles.d/touchbar.conf"
                | "share/touchbar/touchbard.toml"
        )
    ) || p.starts_with("share/touchbar/plugins") && p.components().count() > 4
        || p.starts_with("share/doc/touchbar") && p.components().count() > 3
}
fn hash(file: &mut File) -> Result<String, String> {
    let mut h = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes).map_err(err)?;
        if n == 0 {
            break;
        }
        h.update(&bytes[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn root_fd(root: &Path) -> Result<File, String> {
    let fd = open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(err)?;
    rustix::fs::flock(&fd, FlockOperation::NonBlockingLockExclusive)
        .map_err(|e| format!("staging tree is busy: {e}"))?;
    Ok(fd.into())
}
fn within(root: &impl AsFd, path: &Path, flags: OFlags) -> Result<File, rustix::io::Errno> {
    openat2(
        root,
        path,
        flags | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
        RESOLVE,
    )
    .map(File::from)
}
fn load(root: &File) -> Result<Manifest, String> {
    let file = within(root, Path::new(MANIFEST), OFlags::RDONLY).map_err(err)?;
    if !file.metadata().map_err(err)?.is_file() {
        return Err("manifest must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("install manifest is too large".into());
    }
    let m: Manifest = serde_json::from_slice(&bytes).map_err(err)?;
    if m.version != 1 {
        return Err(format!(
            "unsupported install manifest version {}",
            m.version
        ));
    }
    if prefix(&m.prefix)? != m.prefix || m.files.is_empty() {
        return Err("invalid install manifest".into());
    }
    let mut paths = BTreeSet::new();
    for e in &m.files {
        if !relative(&e.path)
            || !package_path(&m.prefix, &e.path)
            || !paths.insert(e.path.clone())
            || !matches!(e.mode, 0o644 | 0o755)
            || e.sha256.len() != 64
            || !e.sha256.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("invalid or duplicate package path/mode/hash in manifest".into());
        }
    }
    Ok(m)
}
fn verify(root: &File, m: &Manifest) -> Result<(), String> {
    for e in &m.files {
        let mut f = match within(root, &e.path, OFlags::RDONLY) {
            Ok(f) => f,
            Err(rustix::io::Errno::NOENT) => continue,
            Err(e) => return Err(err(e)),
        };
        let meta = f.metadata().map_err(err)?;
        if !meta.is_file()
            || meta.permissions().mode() & 0o777 != e.mode
            || hash(&mut f)? != e.sha256
        {
            return Err(format!(
                "installed file changed; refusing removal/replacement: {}",
                e.path.display()
            ));
        }
    }
    Ok(())
}
fn inventory(root: &Path) -> Result<Vec<PathBuf>, String> {
    fn walk(root: &Path, p: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
        for entry in fs::read_dir(p).map_err(err)? {
            let entry = entry.map_err(err)?;
            let kind = entry.file_type().map_err(err)?;
            if kind.is_dir() {
                walk(root, &entry.path(), files)?
            } else if kind.is_file() {
                files.push(entry.path().strip_prefix(root).map_err(err)?.into())
            } else {
                return Err(format!(
                    "symlink or special file in package tree: {}",
                    entry.path().display()
                ));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort();
    Ok(files)
}
fn destination(root: &Path, out: &Path, a: &Artifacts) -> Result<PathBuf, String> {
    if fs::symlink_metadata(out).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("destdir must not be a symlink".into());
    }
    let out = super::absolute(out)?;
    for source in [
        &root.join("docs"),
        &root.join("packaging"),
        &a.plugins,
        &a.daemon,
        &a.session,
        &a.helper,
        &a.ctl,
    ] {
        let source = source.canonicalize().map_err(err)?;
        if source.starts_with(&out) || out.starts_with(source) {
            return Err("destdir overlaps package inputs".into());
        }
    }
    Ok(out)
}
fn existing(out: &Path) -> Result<Option<File>, String> {
    if !out.exists() {
        return Ok(None);
    }
    let fd = root_fd(out)?;
    if fs::read_dir(out).map_err(err)?.next().is_none() {
        return Ok(Some(fd));
    }
    let m = load(&fd).map_err(|e| format!("refusing unowned staging output: {e}"))?;
    verify(&fd, &m)?;
    let mut expected = m
        .files
        .iter()
        .map(|e| e.path.clone())
        .collect::<BTreeSet<_>>();
    expected.insert(MANIFEST.into());
    if inventory(out)?.into_iter().collect::<BTreeSet<_>>() != expected {
        return Err("staging output has additional files; refusing replacement".into());
    }
    Ok(Some(fd))
}
fn copy(source: &Path, dest: &Path, mode: u32) -> Result<(), String> {
    if !fs::symlink_metadata(source).map_err(err)?.is_file() {
        return Err(format!("not a regular source file: {}", source.display()));
    }
    fs::create_dir_all(dest.parent().ok_or("file needs parent")?).map_err(err)?;
    fs::copy(source, dest).map_err(err)?;
    fs::set_permissions(dest, fs::Permissions::from_mode(mode)).map_err(err)
}
fn tree(source: &Path, dest: &Path) -> Result<(), String> {
    if !fs::symlink_metadata(source).map_err(err)?.is_dir() {
        return Err("source tree must be a directory".into());
    }
    fs::create_dir_all(dest).map_err(err)?;
    for entry in fs::read_dir(source).map_err(err)? {
        let entry = entry.map_err(err)?;
        if entry.file_name() == ".touchbar-built-plugins" {
            continue;
        }
        if entry.file_type().map_err(err)?.is_dir() {
            tree(&entry.path(), &dest.join(entry.file_name()))?
        } else {
            copy(&entry.path(), &dest.join(entry.file_name()), 0o644)?
        }
    }
    Ok(())
}
fn unit_path(path: &Path) -> Result<String, String> {
    Ok(format!(
        "\"{}\"",
        path.to_str()
            .ok_or("unit path is not UTF-8")?
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "$$")
            .replace('%', "%%")
    ))
}
/// Assemble already-built artifacts; binary compilation and plugin building live in build().
pub fn assemble(
    root: &Path,
    out: &Path,
    install_prefix: &Path,
    a: &Artifacts,
) -> Result<PathBuf, String> {
    let prefix = prefix(install_prefix)?;
    let out = destination(root, out, a)?;
    let _old_lock = existing(&out)?;
    let parent = out.parent().ok_or("destdir needs a parent")?;
    fs::create_dir_all(parent).map_err(err)?;
    let temp = tempfile::Builder::new()
        .prefix(".touchbar-install-")
        .tempdir_in(parent)
        .map_err(err)?;
    let staged = temp.path().join("tree");
    fs::create_dir(&staged).map_err(err)?;
    let base = staged.join(prefix.strip_prefix("/").map_err(err)?);
    for (source, target) in [
        (&a.daemon, "lib/touchbar/touchbard"),
        (&a.session, "lib/touchbar/touchbar-session"),
        (&a.helper, "lib/touchbar/touchbar-spawn-helper"),
        (&a.ctl, "bin/touchbarctl"),
    ] {
        copy(source, &base.join(target), 0o755)?;
    }
    for (source, target) in [
        (
            "packaging/udev/90-touchbar.rules",
            "lib/udev/rules.d/90-touchbar.rules",
        ),
        (
            "packaging/udev/99-wstouchbar-isolation.rules",
            "lib/udev/rules.d/99-wstouchbar-isolation.rules",
        ),
        (
            "packaging/tmpfiles/touchbar.conf",
            "lib/tmpfiles.d/touchbar.conf",
        ),
        ("packaging/touchbard.toml", "share/touchbar/touchbard.toml"),
        ("README.md", "share/doc/touchbar/README.md"),
        ("LICENSE-MIT", "share/doc/touchbar/LICENSE-MIT"),
        ("LICENSE-APACHE", "share/doc/touchbar/LICENSE-APACHE"),
    ] {
        copy(&root.join(source), &base.join(target), 0o644)?;
    }
    for (name, target) in [
        ("touchbard.service", "system"),
        ("touchbar-session.service", "user"),
    ] {
        let source = root.join("packaging/systemd").join(name);
        if !fs::symlink_metadata(&source).map_err(err)?.is_file() {
            return Err("unit template must be a regular file".into());
        }
        let text = fs::read_to_string(&source)
            .map_err(err)?
            .replace(
                "@TOUCHBARD@",
                &unit_path(&prefix.join("lib/touchbar/touchbard"))?,
            )
            .replace(
                "@SESSION@",
                &unit_path(&prefix.join("lib/touchbar/touchbar-session"))?,
            );
        let path = base.join("lib/systemd").join(target).join(name);
        fs::create_dir_all(path.parent().unwrap()).map_err(err)?;
        fs::write(&path, text).map_err(err)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).map_err(err)?;
    }
    tree(&a.plugins, &base.join("share/touchbar/plugins"))?;
    tree(&root.join("docs"), &base.join("share/doc/touchbar/docs"))?;
    // Stable modes independent of developer umask, including intermediate directories.
    fn modes(p: &Path) -> Result<(), String> {
        fs::set_permissions(p, fs::Permissions::from_mode(0o755)).map_err(err)?;
        for e in fs::read_dir(p).map_err(err)? {
            let e = e.map_err(err)?;
            if e.file_type().map_err(err)?.is_dir() {
                modes(&e.path())?
            }
        }
        Ok(())
    }
    modes(&staged)?;
    let mut files = Vec::new();
    for path in inventory(&staged)? {
        if !relative(&path) || !package_path(&prefix, &path) {
            return Err("unexpected package path".into());
        }
        let mut f = File::open(staged.join(&path)).map_err(err)?;
        files.push(Entry {
            path,
            mode: f.metadata().map_err(err)?.permissions().mode() & 0o777,
            sha256: hash(&mut f)?,
        });
    }
    let manifest = Manifest {
        version: 1,
        prefix,
        files,
    };
    fs::write(
        staged.join(MANIFEST),
        serde_json::to_vec_pretty(&manifest).map_err(err)?,
    )
    .map_err(err)?;
    fs::set_permissions(staged.join(MANIFEST), fs::Permissions::from_mode(0o644)).map_err(err)?;
    let previous = temp.path().join("previous");
    let replacing = out.exists();
    if replacing {
        fs::rename(&out, &previous).map_err(err)?
    }
    if let Err(e) = fs::rename(&staged, &out) {
        if replacing && let Err(restore) = fs::rename(&previous, &out) {
            return Err(format!(
                "publish failed: {e}; restore failed: {restore}; previous output retained in {}",
                temp.keep().display()
            ));
        }
        return Err(err(e));
    }
    Ok(out.join(MANIFEST))
}
/// Remove only package files whose contents and modes still match their manifest.
pub fn uninstall(path: &Path) -> Result<(), String> {
    if path.file_name() != Some(std::ffi::OsStr::new(MANIFEST)) {
        return Err("use the staging root install manifest".into());
    }
    let root = path.parent().ok_or("manifest needs a parent")?;
    let fd = root_fd(root)?;
    let m = load(&fd)?;
    verify(&fd, &m)?;
    let mut dirs = BTreeSet::new();
    for e in &m.files {
        let parent = e.path.parent().ok_or("file needs parent")?;
        match within(&fd, parent, OFlags::RDONLY | OFlags::DIRECTORY) {
            Ok(parent_fd) => {
                match unlinkat(&parent_fd, e.path.file_name().unwrap(), AtFlags::empty()) {
                    Ok(()) | Err(rustix::io::Errno::NOENT) => {}
                    Err(e) => return Err(err(e)),
                }
            }
            Err(rustix::io::Errno::NOENT) => {}
            Err(e) => return Err(err(e)),
        }
        let mut p = parent;
        while !p.as_os_str().is_empty() {
            dirs.insert(p.to_path_buf());
            p = p.parent().unwrap_or(Path::new(""));
        }
    }
    let mut dirs = dirs.into_iter().collect::<Vec<_>>();
    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for dir in dirs {
        if let Ok(parent) = within(
            &fd,
            dir.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
            OFlags::RDONLY | OFlags::DIRECTORY,
        ) {
            match unlinkat(parent, dir.file_name().unwrap(), AtFlags::REMOVEDIR) {
                Ok(()) | Err(rustix::io::Errno::NOENT | rustix::io::Errno::NOTEMPTY) => {}
                Err(e) => return Err(err(e)),
            }
        }
    }
    unlinkat(&fd, MANIFEST, AtFlags::empty()).map_err(err)
}

/// Build a standalone package tree; no root operations or service-manager calls.
pub fn build(
    root: &Path,
    out: &Path,
    install_prefix: &Path,
    profile: &str,
) -> Result<PathBuf, String> {
    prefix(install_prefix)?;
    if !matches!(profile, "dev" | "release") {
        return Err("profile must be dev or release".into());
    }
    if fs::symlink_metadata(out).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("destdir must not be a symlink".into());
    }
    let out = super::absolute(out)?;
    for source in [
        root.join("plugins"),
        root.join("docs"),
        root.join("packaging"),
    ] {
        let source = source.canonicalize().map_err(err)?;
        if source.starts_with(&out) || out.starts_with(&source) {
            return Err("destdir overlaps package inputs".into());
        }
    }
    drop(existing(&out)?);
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    eprintln!("Building runtime binaries ({profile})...");
    let result = std::process::Command::new(cargo)
        .current_dir(root)
        .args([
            "build",
            "--locked",
            "--profile",
            profile,
            "--message-format=json-render-diagnostics",
            "--bin",
            "touchbard",
            "--bin",
            "touchbar-session",
            "--bin",
            "touchbar-spawn-helper",
            "--bin",
            "touchbarctl",
            "-p",
            "touchbard",
            "-p",
            "touchbar-session",
            "-p",
            "touchbar-effects",
            "-p",
            "touchbarctl",
        ])
        .output()
        .map_err(err)?;
    if !result.status.success() {
        return Err(format!(
            "runtime build failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    let mut binaries = std::collections::BTreeMap::new();
    for line in result.stdout.split(|b| *b == b'\n') {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(line)
            && value["reason"] == "compiler-artifact"
            && let (Some(name), Some(path)) = (
                value["target"]["name"].as_str(),
                value["executable"].as_str(),
            )
        {
            binaries.insert(name.to_owned(), PathBuf::from(path));
        }
    }
    let mut binary = |name: &str| {
        binaries
            .remove(name)
            .ok_or_else(|| format!("Cargo did not produce {name}"))
    };
    let temporary = tempfile::tempdir().map_err(err)?;
    let artifacts = Artifacts {
        daemon: binary("touchbard")?,
        session: binary("touchbar-session")?,
        helper: binary("touchbar-spawn-helper")?,
        ctl: binary("touchbarctl")?,
        plugins: temporary.path().join("plugins"),
    };
    eprintln!("Building runtime plugins...");
    super::build_plugins(root, &artifacts.plugins)?;
    assemble(root, &out, install_prefix, &artifacts)
}
