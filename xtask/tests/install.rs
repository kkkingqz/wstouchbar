use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};
use xtask::install::{Artifacts, assemble, uninstall};

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    out: PathBuf,
    artifacts: Artifacts,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("source");
        for (name, bytes) in [
            (
                "packaging/systemd/touchbard.service",
                "[Service]\nExecStart=@TOUCHBARD@\nExecStopPost=@TOUCHBARD@ --hand-back\n",
            ),
            (
                "packaging/systemd/touchbar-session.service",
                "[Service]\nExecStart=@SESSION@\n",
            ),
            ("packaging/udev/90-touchbar.rules", "# start\n"),
            (
                "packaging/udev/99-wstouchbar-isolation.rules",
                "# isolation\n",
            ),
            (
                "packaging/tmpfiles/touchbar.conf",
                "d /run/touchbar 0755 root root -\n",
            ),
            ("packaging/touchbard.toml", "session-grace-ms=2000\n"),
            ("README.md", "readme"),
            ("LICENSE-MIT", "mit"),
            ("LICENSE-APACHE", "apache"),
            ("docs/hardware-checklist.md", "hardware"),
        ] {
            write(&root.join(name), bytes.as_bytes());
        }
        let bin = dir.path().join("binaries");
        for name in [
            "touchbard",
            "touchbar-session",
            "touchbar-spawn-helper",
            "touchbarctl",
        ] {
            write(&bin.join(name), b"#!/bin/sh\nexit 0\n");
        }
        let plugins = dir.path().join("plugins");
        write(
            &plugins.join("controls/plugin.toml"),
            b"id='controls'\nlogic='logic.wasm'\n",
        );
        write(&plugins.join("controls/logic.wasm"), b"component fixture");
        let artifacts = Artifacts {
            daemon: bin.join("touchbard"),
            session: bin.join("touchbar-session"),
            helper: bin.join("touchbar-spawn-helper"),
            ctl: bin.join("touchbarctl"),
            plugins,
        };
        Self {
            out: dir.path().join("stage"),
            root,
            artifacts,
            _dir: dir,
        }
    }
    fn install(&self) -> PathBuf {
        assemble(&self.root, &self.out, Path::new("/usr"), &self.artifacts).unwrap()
    }
}
fn write(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}
fn files(p: &Path) -> BTreeSet<String> {
    fn walk(root: &Path, p: &Path, result: &mut BTreeSet<String>) {
        for e in fs::read_dir(p).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                walk(root, &e.path(), result);
            } else {
                result.insert(
                    e.path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                );
            }
        }
    }
    let mut result = BTreeSet::new();
    walk(p, p, &mut result);
    result
}
#[test]
fn staged_tree_has_exact_runtime_files_permissions_and_rendered_units() {
    let f = Fixture::new();
    let manifest = f.install();
    let expected = [
        ".touchbar-install-manifest.json",
        "usr/lib/touchbar/touchbard",
        "usr/lib/touchbar/touchbar-session",
        "usr/lib/touchbar/touchbar-spawn-helper",
        "usr/bin/touchbarctl",
        "usr/lib/systemd/system/touchbard.service",
        "usr/lib/systemd/user/touchbar-session.service",
        "usr/lib/udev/rules.d/90-touchbar.rules",
        "usr/lib/udev/rules.d/99-wstouchbar-isolation.rules",
        "usr/lib/tmpfiles.d/touchbar.conf",
        "usr/share/touchbar/touchbard.toml",
        "usr/share/touchbar/plugins/controls/plugin.toml",
        "usr/share/touchbar/plugins/controls/logic.wasm",
        "usr/share/doc/touchbar/README.md",
        "usr/share/doc/touchbar/LICENSE-MIT",
        "usr/share/doc/touchbar/LICENSE-APACHE",
        "usr/share/doc/touchbar/docs/hardware-checklist.md",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(files(&f.out), expected);
    for p in files(&f.out) {
        let executable = p.starts_with("usr/lib/touchbar/") || p == "usr/bin/touchbarctl";
        assert_eq!(
            fs::metadata(f.out.join(&p)).unwrap().permissions().mode() & 0o777,
            if executable { 0o755 } else { 0o644 },
            "{p}"
        );
    }
    let unit = fs::read_to_string(f.out.join("usr/lib/systemd/system/touchbard.service")).unwrap();
    assert!(unit.contains("ExecStart=\"/usr/lib/touchbar/touchbard\"\n"));
    let m: serde_json::Value = serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
    assert_eq!(m["version"], 1);
    assert_eq!(m["files"].as_array().unwrap().len(), 16);
    assert!(
        !serde_json::to_string(&m)
            .unwrap()
            .contains(f._dir.path().to_str().unwrap())
    );
}
#[test]
fn uninstall_removes_only_recorded_files_and_preserves_user_additions() {
    let f = Fixture::new();
    let manifest = f.install();
    write(&f.out.join("usr/share/touchbar/note.txt"), b"keep");
    uninstall(&manifest).unwrap();
    assert_eq!(
        files(&f.out),
        BTreeSet::from(["usr/share/touchbar/note.txt".into()])
    );
}
#[test]
fn modified_file_aborts_uninstall_before_any_removal() {
    let f = Fixture::new();
    let manifest = f.install();
    write(&f.out.join("usr/lib/touchbar/touchbard"), b"user change");
    let before = files(&f.out);
    assert!(uninstall(&manifest).is_err());
    assert_eq!(files(&f.out), before);
    assert_eq!(
        fs::read(f.out.join("usr/lib/touchbar/touchbard")).unwrap(),
        b"user change"
    );
}
#[test]
fn symlink_ancestor_cannot_redirect_uninstall_outside_the_stage() {
    let f = Fixture::new();
    let manifest = f.install();
    let original = f._dir.path().join("original");
    fs::rename(f.out.join("usr/lib"), &original).unwrap();
    symlink(&original, f.out.join("usr/lib")).unwrap();
    assert!(uninstall(&manifest).is_err());
    assert!(original.join("touchbar/touchbard").exists());
    assert!(manifest.exists());
}
#[test]
fn forged_manifest_paths_and_unknown_versions_are_rejected() {
    for (field, value) in [
        ("version", serde_json::json!(99)),
        ("path", serde_json::json!("../outside")),
        ("path", serde_json::json!("/etc/passwd")),
        ("path", serde_json::json!("usr/bin/unrelated")),
    ] {
        let f = Fixture::new();
        let manifest = f.install();
        let mut m: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        if field == "version" {
            m[field] = value;
        } else {
            m["files"][0][field] = value;
        }
        fs::write(&manifest, serde_json::to_vec(&m).unwrap()).unwrap();
        assert!(uninstall(&manifest).is_err());
        assert!(f.out.join("usr/lib/touchbar/touchbard").exists());
    }
}
#[test]
fn rebuild_is_atomic_and_refuses_foreign_or_modified_output() {
    let f = Fixture::new();
    f.install();
    write(&f.artifacts.daemon, b"new daemon");
    f.install();
    assert_eq!(
        fs::read(f.out.join("usr/lib/touchbar/touchbard")).unwrap(),
        b"new daemon"
    );
    fs::remove_file(&f.artifacts.ctl).unwrap();
    assert!(assemble(&f.root, &f.out, Path::new("/usr"), &f.artifacts).is_err());
    assert_eq!(
        fs::read(f.out.join("usr/lib/touchbar/touchbard")).unwrap(),
        b"new daemon"
    );
    write(&f.artifacts.ctl, b"restored ctl");
    write(&f.out.join("unrelated"), b"keep");
    assert!(assemble(&f.root, &f.out, Path::new("/usr"), &f.artifacts).is_err());
    let g = Fixture::new();
    write(&g.out.join("user-data"), b"keep");
    assert!(assemble(&g.root, &g.out, Path::new("/usr"), &g.artifacts).is_err());
    assert_eq!(fs::read(g.out.join("user-data")).unwrap(), b"keep");
}
#[test]
fn prefix_is_logical_and_cannot_traverse_or_inject_unit_directives() {
    let f = Fixture::new();
    for bad in ["relative", "/usr/../etc", "/usr\nExecStart=/bin/false"] {
        assert!(assemble(&f.root, &f.out, Path::new(bad), &f.artifacts).is_err());
        assert!(!f.out.exists());
    }
    let manifest = assemble(&f.root, &f.out, Path::new("/opt/Touch Bar$%"), &f.artifacts).unwrap();
    let unit = fs::read_to_string(
        f.out
            .join("opt/Touch Bar$%/lib/systemd/system/touchbard.service"),
    )
    .unwrap();
    assert!(unit.contains("ExecStart=\"/opt/Touch Bar$$%%/lib/touchbar/touchbard\""));
    uninstall(&manifest).unwrap();
    assert!(files(&f.out).is_empty());
    assert!(
        !f.out.join("opt").exists(),
        "empty package directories left behind"
    );
}
#[test]
fn binary_uninstall_uses_the_manifest_and_requires_an_explicit_staging_destination() {
    let f = Fixture::new();
    let manifest = f.install();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["uninstall", "--manifest"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(files(&f.out).is_empty());
    for args in [
        vec!["install"],
        vec!["install", "--prefix=/"],
        vec!["uninstall"],
        vec!["install", "--destdir=/tmp/example", "--profile=unknown"],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
    }
}
#[test]
fn option_without_a_value_is_rejected_before_any_build() {
    let dir = tempfile::tempdir().unwrap();
    let cargo = dir.path().join("cargo");
    let marker = dir.path().join("called");
    fs::write(&cargo, "#!/bin/sh\n: > \"$MARKER\"\nexit 89\n").unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["install", "--destdir", "--profile=dev"])
        .env("CARGO", &cargo)
        .env("MARKER", &marker)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!marker.exists(), "invalid CLI started a runtime build");
}
