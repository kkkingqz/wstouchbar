use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn wait_log(child: &mut Child, path: &Path, needle: &str, count: usize) {
    let start = Instant::now();
    loop {
        let log = fs::read_to_string(path).unwrap();
        if log.matches(needle).count() >= count {
            return;
        }
        assert!(child.try_wait().unwrap().is_none(), "session exited: {log}");
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "missing {needle}: {log}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn headless_binary_keeps_running_with_null_backend_reloads_and_handles_sigterm() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("session.log");
    let output = fs::File::create(&log).unwrap();
    let config = dir.path().join("config/touchbar");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(dir.path().join("plugins")).unwrap();
    fs::create_dir_all(dir.path().join("data/touchbar/plugins")).unwrap();
    let mut child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_touchbar-session"))
            .args([
                "--headless",
                "400x60@6.2ppmm",
                "--effects=fake",
                "--desktop=null",
                "--plugins",
            ])
            .arg(dir.path().join("plugins"))
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_STATE_HOME", dir.path().join("state"))
            .env("XDG_CACHE_HOME", dir.path().join("cache"))
            .env("XDG_DATA_HOME", dir.path().join("data"))
            .env_remove("TOUCHBAR_LOG")
            .env_remove("JOURNAL_STREAM")
            .stdout(Stdio::null())
            .stderr(output)
            .spawn()
            .unwrap(),
    );
    wait_log(&mut child.0, &log, "catalog_loaded", 1);
    fs::write(config.join("config.toml"), "log='debug'\n").unwrap();
    wait_log(&mut child.0, &log, "catalog_loaded", 2);
    rustix::process::kill_process(
        rustix::process::Pid::from_raw(i32::try_from(child.0.id()).unwrap()).unwrap(),
        rustix::process::Signal::TERM,
    )
    .unwrap();
    let start = Instant::now();
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success(), "{}", fs::read_to_string(log).unwrap());
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(10));
    }
}
