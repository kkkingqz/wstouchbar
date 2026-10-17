use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use touchbar_effects::spawn::*;
use touchbar_model::*;
static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "wstouchbar-spawn-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn owner(n: u64) -> InstanceId {
    InstanceId {
        number: n,
        generation: Generation(1),
    }
}
fn pool() -> SpawnPool {
    SpawnPool::new(env!("CARGO_BIN_EXE_touchbar-spawn-helper").into(), 4, 64)
}
fn spec(args: &[&str]) -> SpawnSpec {
    let mut argv = vec![env!("CARGO_BIN_EXE_touchbar-effects-test-helper").into()];
    argv.extend(args.iter().map(|a| a.to_string()));
    SpawnSpec::new(argv)
}
fn finish(process: &Process) -> Result<SpawnResult, EffectError> {
    futures_lite::future::block_on(async {
        futures_lite::future::race(
            async {
                loop {
                    if let ProcessEvent::Exited(result) = process.events.recv().await.unwrap() {
                        return result;
                    }
                }
            },
            async {
                async_io::Timer::after(Duration::from_secs(6)).await;
                panic!("process never completed")
            },
        )
        .await
    })
}
fn is_running(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| {
        s.rsplit_once(") ")
            .is_some_and(|(_, rest)| !rest.starts_with('Z'))
    })
}
fn wait_pids(path: &std::path::Path) -> Vec<u32> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(s) = fs::read_to_string(path) {
            let ids: Vec<_> = s.split_whitespace().map(|s| s.parse().unwrap()).collect();
            if ids.len() == 2 {
                return ids;
            }
        }
        assert!(Instant::now() < deadline, "no pid file");
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn argv_environment_stdin_and_exit_code_are_preserved_without_shell() {
    let pool = pool();
    let mut spec = spec(&["echo", "$HOME; printf surprise"]);
    spec.env
        .insert("TOUCHBAR_TEST_VALUE".into(), "override".into());
    spec.stdin = Some("payload".into());
    let process = pool.start(owner(1), spec, false, Some(3000)).unwrap();
    let result = finish(&process).unwrap();
    assert_eq!(result.status, 7);
    let output: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(output["arg"], "$HOME; printf surprise");
    assert_eq!(output["env"], "override");
    assert_eq!(output["stdin"], "payload");
    assert_eq!(result.stderr, "error stream");
    assert!(!result.truncated);
}
#[test]
fn output_is_bounded_and_both_streams_are_drained() {
    let pool = pool();
    let mut s = spec(&["flood"]);
    s.max_output = 257;
    let process = pool.start(owner(1), s, false, Some(3000)).unwrap();
    let result = finish(&process).unwrap();
    assert_eq!(result.status, 0);
    assert_eq!(result.stdout.len(), 257);
    assert_eq!(result.stderr.len(), 257);
    assert!(result.truncated);
}
#[test]
fn lines_are_delivered_and_exit_is_reported() {
    let pool = pool();
    let process = pool
        .start(owner(1), spec(&["lines"]), true, Some(3000))
        .unwrap();
    let mut lines = vec![];
    loop {
        match process.events.recv_blocking().unwrap() {
            ProcessEvent::Line(s) => lines.push(s),
            ProcessEvent::Exited(result) => {
                assert_eq!(result.unwrap().status, 0);
                break;
            }
        }
    }
    assert_eq!(lines, ["one", "two", "tail"]);
}
#[test]
fn cancellation_and_timeout_kill_the_process_group_including_term_ignoring_child() {
    for timed in [false, true] {
        let t = Temp::new();
        let path = t.0.join("pids");
        let pool = pool();
        let process = pool
            .start(
                owner(1),
                spec(&["descendants", path.to_str().unwrap()]),
                false,
                if timed { Some(300) } else { None },
            )
            .unwrap();
        let pids = wait_pids(&path);
        assert!(pids.iter().all(|p| is_running(*p)));
        let started = Instant::now();
        if !timed {
            process.cancel();
        }
        assert_eq!(
            finish(&process),
            Err(if timed {
                EffectError::Timeout
            } else {
                EffectError::Cancelled
            })
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(
            pids.iter().all(|p| !is_running(*p)),
            "descendant remains alive: {pids:?}"
        );
    }
}
#[test]
fn quotas_cover_active_jobs_and_release_after_reaping() {
    let pool = pool();
    let processes: Vec<_> = (0..4)
        .map(|_| pool.start(owner(1), spec(&["sleep"]), false, None).unwrap())
        .collect();
    assert!(pool.start(owner(1), spec(&["sleep"]), false, None).is_err());
    let other = pool.start(owner(2), spec(&["sleep"]), false, None).unwrap();
    for p in &processes {
        p.cancel();
        let _ = finish(p);
    }
    other.cancel();
    let _ = finish(&other);
    let p = pool
        .start(owner(1), spec(&["lines"]), false, Some(3000))
        .unwrap();
    assert_eq!(finish(&p).unwrap().status, 0);
    let limited = SpawnPool::new(env!("CARGO_BIN_EXE_touchbar-spawn-helper").into(), 4, 1);
    let first = limited
        .start(owner(1), spec(&["sleep"]), false, None)
        .unwrap();
    assert!(
        limited
            .start(owner(2), spec(&["sleep"]), false, None)
            .is_err()
    );
    first.cancel();
    let _ = finish(&first);
}
#[test]
fn abrupt_parent_death_kills_descendants() {
    let t = Temp::new();
    let path = t.0.join("pids");
    let mut parent = std::process::Command::new(env!("CARGO_BIN_EXE_touchbar-effects-test-helper"))
        .args([
            "spawn-parent",
            env!("CARGO_BIN_EXE_touchbar-spawn-helper"),
            path.to_str().unwrap(),
        ])
        .spawn()
        .unwrap();
    let pids = wait_pids(&path);
    parent.kill().unwrap();
    parent.wait().unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    while pids.iter().any(|p| is_running(*p)) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        pids.iter().all(|p| !is_running(*p)),
        "orphaned descendants: {pids:?}"
    );
}

#[test]
fn cancellation_remains_effective_after_helper_exit_with_line_backpressure() {
    let pool = pool();
    let process = pool
        .start(owner(1), spec(&["many-lines"]), true, None)
        .unwrap();
    std::thread::sleep(Duration::from_millis(150));
    process.cancel();
    assert_eq!(finish(&process), Err(EffectError::Cancelled));
}
#[test]
fn unexpected_supervisor_death_cleans_its_remaining_group() {
    let t = Temp::new();
    let path = t.0.join("pids");
    let pool = pool();
    let process = pool
        .start(
            owner(1),
            spec(&["descendants", path.to_str().unwrap()]),
            false,
            Some(500),
        )
        .unwrap();
    let pids = wait_pids(&path);
    let stat = fs::read_to_string(format!("/proc/{}/stat", pids[0])).unwrap();
    let helper: i32 = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let helper = rustix::process::Pid::from_raw(helper).unwrap();
    rustix::process::kill_process(helper, rustix::process::Signal::KILL).unwrap();
    let result = finish(&process);
    let leaked = pids.iter().any(|p| is_running(*p));
    if leaked {
        let _ = rustix::process::kill_process_group(helper, rustix::process::Signal::KILL);
    }
    assert!(
        matches!(result, Err(EffectError::Failed(_))),
        "unexpected result {result:?}"
    );
    assert!(!leaked, "supervisor death left its group alive");
}
#[test]
fn shared_process_quota_follows_remaining_owners() {
    let pool = pool();
    let owners = [owner(1), owner(2)].into();
    let processes: Vec<_> = (0..4)
        .map(|_| {
            pool.start_shared(&owners, spec(&["sleep"]), false, None)
                .unwrap()
        })
        .collect();
    assert!(pool.start(owner(1), spec(&["sleep"]), false, None).is_err());
    for process in &processes {
        process.set_owners([owner(2)].into());
    }
    let released = pool
        .start(owner(1), spec(&["lines"]), false, Some(3000))
        .unwrap();
    assert_eq!(finish(&released).unwrap().status, 0);
    assert!(pool.start(owner(2), spec(&["sleep"]), false, None).is_err());
    for process in &processes {
        process.cancel();
        let _ = finish(process);
    }
}

#[test]
fn setsid_descendants_are_reaped_and_release_quota_on_exit_cancel_and_timeout() {
    for mode in ["escaped-exit", "escaped", "timeout"] {
        let t = Temp::new();
        let path = t.0.join("pids");
        let pool = SpawnPool::new(env!("CARGO_BIN_EXE_touchbar-spawn-helper").into(), 1, 1);
        let process = pool
            .start(
                owner(1),
                spec(&[
                    if mode == "timeout" { "escaped" } else { mode },
                    path.to_str().unwrap(),
                ]),
                false,
                (mode == "timeout").then_some(300),
            )
            .unwrap();
        let pids = wait_pids(&path);
        if mode == "escaped" {
            process.cancel();
        }
        let result = finish(&process);
        if mode == "escaped-exit" {
            assert_eq!(result.unwrap().status, 0);
        } else {
            assert_eq!(
                result,
                Err(if mode == "timeout" {
                    EffectError::Timeout
                } else {
                    EffectError::Cancelled
                })
            );
        }
        assert!(
            pids.iter().all(|pid| !is_running(*pid)),
            "escaped descendants: {pids:?}"
        );
        assert_eq!(pool.active(), 0);
    }
}

#[test]
fn silent_spawn_does_not_periodically_wake_worker_or_helper() {
    const CHILD: &str = "TOUCHBAR_IDLE_WAKE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "silent_spawn_does_not_periodically_wake_worker_or_helper",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(result.success());
        return;
    }
    let pool = pool();
    let process = pool.start(owner(1), spec(&["idle"]), true, None).unwrap();
    let ProcessEvent::Line(pid) = process.events.recv_blocking().unwrap() else {
        panic!("no pid")
    };
    let target: u32 = pid.parse().unwrap();
    let status = fs::read_to_string(format!("/proc/{target}/status")).unwrap();
    let helper: u32 = status
        .lines()
        .find_map(|line| line.strip_prefix("PPid:\t"))
        .unwrap()
        .parse()
        .unwrap();
    let worker = fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            fs::read_to_string(entry.path().join("comm"))
                .is_ok_and(|name| name.trim() == "touchbar-spawn")
        })
        .unwrap()
        .path();
    fn switches(path: &std::path::Path) -> u64 {
        fs::read_to_string(path.join("status"))
            .unwrap()
            .lines()
            .filter_map(|line| line.strip_prefix("voluntary_ctxt_switches:\t"))
            .map(|value| value.parse::<u64>().unwrap())
            .sum()
    }
    std::thread::sleep(Duration::from_millis(50));
    let helper = PathBuf::from(format!("/proc/{helper}"));
    let before = (switches(&worker), switches(&helper));
    std::thread::sleep(Duration::from_millis(250));
    let wakeups = (switches(&worker) - before.0, switches(&helper) - before.1);
    process.cancel();
    let _ = finish(&process);
    assert!(
        wakeups.0 < 10 && wakeups.1 < 10,
        "idle wakeups in 250ms: {wakeups:?}"
    );
}

#[test]
fn abrupt_parent_death_kills_setsid_descendants() {
    let t = Temp::new();
    let path = t.0.join("pids");
    let mut parent = std::process::Command::new(env!("CARGO_BIN_EXE_touchbar-effects-test-helper"))
        .args([
            "spawn-parent",
            env!("CARGO_BIN_EXE_touchbar-spawn-helper"),
            path.to_str().unwrap(),
            "escaped",
        ])
        .spawn()
        .unwrap();
    let pids = wait_pids(&path);
    parent.kill().unwrap();
    parent.wait().unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    while pids.iter().any(|pid| is_running(*pid)) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    for pid in &pids {
        if is_running(*pid) {
            rustix::process::kill_process(
                rustix::process::Pid::from_raw(i32::try_from(*pid).unwrap()).unwrap(),
                rustix::process::Signal::KILL,
            )
            .unwrap();
            panic!("escaped child survived parent death");
        }
    }
}

#[test]
fn dead_helper_cannot_hold_quota_forever_through_inherited_pipes() {
    let t = Temp::new();
    let path = t.0.join("pids");
    let pool = pool();
    let process = pool
        .start(
            owner(1),
            spec(&["escaped", path.to_str().unwrap()]),
            false,
            None,
        )
        .unwrap();
    let pids = wait_pids(&path);
    let status = fs::read_to_string(format!("/proc/{}/status", pids[0])).unwrap();
    let helper = status
        .lines()
        .find_map(|line| line.strip_prefix("PPid:\t"))
        .unwrap()
        .parse::<i32>()
        .unwrap();
    rustix::process::kill_process(
        rustix::process::Pid::from_raw(helper).unwrap(),
        rustix::process::Signal::KILL,
    )
    .unwrap();
    let result = std::panic::catch_unwind(|| finish(&process));
    // A killed subreaper cannot collect an escaped child; fixture owns its cleanup.
    for pid in pids {
        if let Some(pid) = rustix::process::Pid::from_raw(i32::try_from(pid).unwrap()) {
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
        }
    }
    assert!(result.unwrap().is_err());
    assert_eq!(pool.active(), 0);
}
