use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "wstouchbar-xtask-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let f = Self { dir };
        f.program("cargo", "printf '%s\\n' \"$@\" > \"$CARGO_MARKER\"\nexit 7");
        f
    }
    fn program(&self, name: &str, body: &str) {
        let path = self.dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(args)
            .env("PATH", &self.dir)
            .env("CARGO", self.dir.join("cargo"))
            .env("CARGO_MARKER", self.dir.join("called"))
            .output()
            .unwrap()
    }
    fn called(&self) -> Option<String> {
        fs::read_to_string(self.dir.join("called")).ok()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}
#[test]
fn missing_gpu_library_fails_before_running_tests() {
    let f = Fixture::new();
    f.program(
        "ldconfig",
        "printf '%s\\n' 'libEGL.so.1 (libc6) => /lib/libEGL.so.1'",
    );
    let out = f.run(&["test-env", "gpu"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("libGLESv2"));
    assert_eq!(f.called(), None);
}
#[test]
fn gpu_dispatches_only_its_ignored_tests_and_propagates_failure() {
    let f = Fixture::new();
    f.program("ldconfig", "printf '%s\\n' 'libEGL.so.1 (libc6) => /lib/libEGL.so.1' 'libGLESv2.so.2 (libc6) => /lib/libGLESv2.so.2'");
    let out = f.run(&["test-env", "gpu"]);
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(
        f.called().as_deref(),
        Some("test\n--workspace\n--locked\nenv_gpu_\n--\n--ignored\n--nocapture\n")
    );
}
#[test]
fn service_test_suites_use_private_environment_tools_and_distinct_filters() {
    for (kind, tools, expected) in [
        (
            "dbus",
            vec!["dbus-daemon"],
            "test\n--workspace\n--locked\nenv_dbus_\n--\n--ignored\n--nocapture\n",
        ),
        (
            "pipewire",
            vec!["pipewire", "wireplumber"],
            "test\n--workspace\n--locked\nenv_pipewire_\n--\n--ignored\n--nocapture\n",
        ),
    ] {
        let f = Fixture::new();
        for tool in tools {
            f.program(tool, "exit 0");
        }
        let out = f.run(&["test-env", kind]);
        assert_eq!(out.status.code(), Some(7));
        assert_eq!(f.called().as_deref(), Some(expected));
    }
}
#[test]
fn missing_wireplumber_is_an_error_and_runs_no_tests() {
    let f = Fixture::new();
    f.program("pipewire", "exit 0");
    let out = f.run(&["test-env", "pipewire"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("wireplumber"));
    assert_eq!(f.called(), None);
}
#[test]
fn failed_environment_probe_runs_no_tests() {
    let f = Fixture::new();
    f.program("dbus-daemon", "exit 9");
    assert!(!f.run(&["test-env", "dbus"]).status.success());
    assert_eq!(f.called(), None);
}
#[test]
fn invalid_command_never_runs_tests() {
    let f = Fixture::new();
    for args in [
        vec!["test-env", "unknown"],
        vec!["test-env"],
        vec!["test-env", "gpu", "extra"],
    ] {
        assert_eq!(f.run(&args).status.code(), Some(2));
    }
    assert_eq!(f.called(), None);
}
