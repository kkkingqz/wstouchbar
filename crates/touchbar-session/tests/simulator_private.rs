//! Private compositor integration. Run explicitly; never connects to the ambient desktop.
use std::{
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
struct Group(Child);
impl Drop for Group {
    fn drop(&mut self) {
        if let Some(pid) = rustix::process::Pid::from_raw(i32::try_from(self.0.id()).unwrap()) {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
        let _ = self.0.wait();
    }
}
fn wait_for(child: &mut Child, log: &Path, check: impl Fn(&str) -> bool) {
    let start = Instant::now();
    loop {
        let text = fs::read_to_string(log).unwrap();
        if check(&text) {
            return;
        }
        assert!(child.try_wait().unwrap().is_none(), "child exited: {text}");
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "timed out: {text}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
#[ignore = "requires Weston headless and Mesa EGL in a private environment"]
fn env_wayland_two_windows_present_gl_shm_frames_and_shutdown_at_two_scales() {
    for scale in [1, 2] {
        let dir = tempfile::tempdir().unwrap();
        let runtime = dir.path().join("runtime");
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let weston_log = dir.path().join("weston.log");
        let mut weston = Group(
            Command::new("weston")
                .args([
                    "--backend=headless",
                    "--renderer=pixman",
                    "--idle-time=0",
                    "--no-config",
                    "--socket=wayland-private",
                ])
                .arg(format!("--scale={scale}"))
                .env("XDG_RUNTIME_DIR", &runtime)
                .env_remove("WAYLAND_DISPLAY")
                .env_remove("DISPLAY")
                .stdout(Stdio::null())
                .stderr(fs::File::create(&weston_log).unwrap())
                .process_group(0)
                .spawn()
                .unwrap(),
        );
        wait_for(&mut weston.0, &weston_log, |_| {
            runtime.join("wayland-private").exists()
        });
        fs::create_dir_all(dir.path().join("config/touchbar")).unwrap();
        fs::create_dir_all(dir.path().join("data/touchbar/plugins")).unwrap();
        let log = dir.path().join("session.log");
        let mut session = Group(
            Command::new(env!("CARGO_BIN_EXE_touchbar-session"))
                .args([
                    "--simulator",
                    "--surface",
                    "touchbar=401x61@6.2ppmm",
                    "--surface",
                    "window:test=601x121@4ppmm",
                    "--desktop=fake",
                    "--effects=fake",
                ])
                .env("XDG_RUNTIME_DIR", &runtime)
                .env("WAYLAND_DISPLAY", "wayland-private")
                .env("XDG_CONFIG_HOME", dir.path().join("config"))
                .env("XDG_DATA_HOME", dir.path().join("data"))
                .env("XDG_STATE_HOME", dir.path().join("state"))
                .env("XDG_CACHE_HOME", dir.path().join("cache"))
                .env("TOUCHBAR_LOG", "debug")
                .env("EGL_PLATFORM", "surfaceless")
                .env("LIBGL_ALWAYS_SOFTWARE", "1")
                .env_remove("JOURNAL_STREAM")
                .stdout(Stdio::null())
                .stderr(fs::File::create(&log).unwrap())
                .process_group(0)
                .spawn()
                .unwrap(),
        );
        wait_for(&mut session.0, &log, |text| {
            text.matches("simulator_surface_ready").count() == 2
                && text.matches("simulator_frame_presented").count() >= 2
        });
        rustix::process::kill_process(
            rustix::process::Pid::from_raw(i32::try_from(session.0.id()).unwrap()).unwrap(),
            rustix::process::Signal::TERM,
        )
        .unwrap();
        let start = Instant::now();
        loop {
            if let Some(status) = session.0.try_wait().unwrap() {
                assert!(status.success(), "{}", fs::read_to_string(&log).unwrap());
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "{}",
                fs::read_to_string(&log).unwrap()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
