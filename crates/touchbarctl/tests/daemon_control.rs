use serde_json::Value;
use std::{collections::BTreeSet, io, os::fd::OwnedFd, path::Path, rc::Rc};
use touchbar_model::*;
use touchbard::{
    Command, Event,
    config::Config,
    runtime::{Drivers, Runtime},
};

struct FakeDrivers {
    initial: Option<Vec<Event>>,
    stop: async_channel::Receiver<()>,
}
impl Drivers for FakeDrivers {
    fn dispatch(&mut self, command: Command) -> Vec<Event> {
        match command {
            Command::AcquireInhibitor { generation } => {
                vec![Event::InhibitorAcquired { generation }]
            }
            _ => vec![],
        }
    }
    fn watch_keys(&mut self, _: BTreeSet<WatchKey>) -> Vec<Event> {
        vec![]
    }
    fn buffers(&self, _: u64) -> Option<&[OwnedFd]> {
        None
    }
    fn reload(&mut self) -> io::Result<Config> {
        Ok(Config::default())
    }
    fn status(&self) -> Value {
        serde_json::json!({"fake":true})
    }
    async fn next(&mut self) -> io::Result<Vec<Event>> {
        if let Some(events) = self.initial.take() {
            return Ok(events);
        }
        let _ = self.stop.recv().await;
        Ok(vec![Event::Shutdown])
    }
    async fn finish(&mut self, _: std::time::Instant) -> io::Result<()> {
        Ok(())
    }
}
fn cli(runtime: Option<&Path>, daemon: &Path, args: &[&str]) -> std::process::Output {
    let mut process = std::process::Command::new(env!("CARGO_BIN_EXE_touchbarctl"));
    process.args(args).arg("--daemon-socket").arg(daemon);
    if let Some(runtime) = runtime {
        process.env("XDG_RUNTIME_DIR", runtime);
    } else {
        process.env_remove("XDG_RUNTIME_DIR");
    }
    process.output().unwrap()
}
fn ok(output: &std::process::Output) -> &str {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::str::from_utf8(&output.stdout).unwrap()
}
fn json(output: &std::process::Output) -> Value {
    serde_json::from_str(ok(output)).unwrap()
}

// Real H11 loop and Session control. The CLI is a separate process and both Unix
// sockets enforce SO_PEERCRED. No Linux device, session bus, or GL is initialized.
fn fixture(with_daemon: bool, body: impl FnOnce(&Path, &Path) + Send + 'static) {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let runtime = dir.path().join("user");
        std::fs::create_dir(&runtime).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut server = touchbar_control::Server::bind(&runtime, false).unwrap();
        let desktop = Rc::new(touchbar_desktop::NullBackend::default());
        let (effects, _) = touchbar_effects::Engine::fake(desktop.clone());
        let mut session = touchbar_session::Session::new(
            touchbar_session::SessionOptions::new(
                dir.path().join("state"),
                touchbar_loader::UserConfig::default(),
            ),
            touchbar_session::ManualClock::default(),
            Box::<touchbar_logic_runtime::MemoryStore>::default(),
            desktop,
            effects,
        )
        .unwrap();
        for (number, name, kind) in [
            (1, "touchbar", SurfaceKind::Touchbar),
            (2, "window:other", SurfaceKind::Window),
        ] {
            let mut info = touchbarctl::parse_surface("400x60@8ppmm").unwrap();
            info.id = SurfaceId(number);
            info.name = name.parse().unwrap();
            info.kind = kind;
            let (surface, _) = touchbar_session::surface::HeadlessSurface::new(info);
            session.add_surface(Box::new(surface)).unwrap();
        }
        session.pump().unwrap();
        let uid = rustix::process::getuid().as_raw();
        let (stop_tx, stop_rx) = async_channel::bounded(1);
        let daemon_dir = dir.path().join("daemon");
        let mut daemon = if with_daemon {
            std::fs::create_dir(&daemon_dir).unwrap();
            Some(
                Runtime::new(
                    Config::default(),
                    false,
                    &daemon_dir,
                    FakeDrivers {
                        initial: Some(vec![
                            Event::Start {
                                preparing_sleep: false,
                            },
                            Event::Session(Some(touchbard::Session {
                                uid,
                                id: "graphical".into(),
                                user_display: Some("graphical".into()),
                                active: true,
                                state: "active".into(),
                                class: "user".into(),
                                kind: "wayland".into(),
                                locked: false,
                            })),
                            Event::Device(None),
                        ]),
                        stop: stop_rx,
                    },
                )
                .unwrap(),
            )
        } else {
            None
        };
        let socket = daemon_dir.join("control.sock");
        let (done_tx, done_rx) = async_channel::bounded(1);
        let child = std::thread::spawn(move || {
            body(&runtime, &socket);
            let _ = stop_tx.send_blocking(());
            done_tx.send_blocking(()).unwrap();
        });
        let serving = async {
            loop {
                let incoming = server.next().await.unwrap().unwrap();
                let (reply, _) = session.control(incoming.request.command.clone(), None);
                server.respond(incoming, reply).unwrap();
            }
        };
        let finished = async {
            let _ = done_rx.recv().await;
        };
        let session_loop = futures_lite::future::or(serving, finished);
        let daemon_loop = async {
            if let Some(daemon) = &mut daemon {
                daemon.run().await.unwrap();
            }
        };
        futures_lite::future::zip(daemon_loop, session_loop).await;
        child.join().unwrap();
    });
}
#[test]
fn binary_groups_both_sources_and_changes_daemon_recovery_over_real_sockets() {
    fixture(true, |runtime, daemon| {
        let status = json(&cli(Some(runtime), daemon, &["status", "--json"]));
        assert_eq!(status["session"]["available"], true);
        assert_eq!(status["daemon"]["available"], true);
        let surfaces = status["surfaces"].as_array().unwrap();
        assert_eq!(surfaces.len(), 2);
        let touchbar = surfaces.iter().find(|s| s["name"] == "touchbar").unwrap();
        assert_eq!(touchbar["session"]["info"]["width_px"], 400);
        assert_eq!(
            touchbar["daemon"]["owner"]["uid"],
            rustix::process::getuid().as_raw()
        );
        assert!(
            surfaces
                .iter()
                .find(|s| s["name"] == "window:other")
                .unwrap()["daemon"]
                .is_null()
        );
        ok(&cli(None, daemon, &["native"]));
        assert_eq!(
            json(&cli(Some(runtime), daemon, &["status", "--json"]))["daemon"]["status"]["recovery"],
            true
        );
        ok(&cli(None, daemon, &["display"]));
        assert_eq!(
            json(&cli(Some(runtime), daemon, &["status", "--json"]))["daemon"]["status"]["recovery"],
            false
        );
        let reload = cli(None, daemon, &["reload", "--daemon"]);
        if rustix::process::getuid().is_root() {
            ok(&reload);
        } else {
            assert!(!reload.status.success());
            assert!(String::from_utf8_lossy(&reload.stderr).contains("not permitted"));
        }
        let daemon_only = json(&cli(None, daemon, &["status", "--json"]));
        assert_eq!(daemon_only["session"]["available"], false);
        assert!(
            daemon_only["session"]["error"]
                .as_str()
                .unwrap()
                .contains("XDG_RUNTIME_DIR")
        );
        assert_eq!(daemon_only["daemon"]["available"], true);
        assert_eq!(daemon_only["surfaces"][0]["name"], "touchbar");
        let text = cli(Some(runtime), daemon, &["status"]);
        let text = ok(&text);
        assert!(text.contains("Recovery: false"), "{text}");
        assert!(text.contains("Owner:"), "{text}");
        assert!(text.contains("window:other"), "{text}");
    });
}
#[test]
fn unavailable_daemon_does_not_hide_session_or_fail_status() {
    fixture(false, |runtime, daemon| {
        let status = json(&cli(Some(runtime), daemon, &["status", "--json"]));
        assert_eq!(status["session"]["available"], true);
        assert_eq!(status["daemon"]["available"], false);
        assert!(status["daemon"]["error"].is_string());
        assert_eq!(status["surfaces"].as_array().unwrap().len(), 2);
        assert!(ok(&cli(Some(runtime), daemon, &["status"])).contains("Daemon unavailable:"));
    });
}
#[test]
fn both_sources_unavailable_are_explicit_and_mutating_commands_still_fail() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("missing.sock");
    let status = json(&cli(None, &socket, &["status", "--json"]));
    assert_eq!(status["session"]["available"], false);
    assert_eq!(status["daemon"]["available"], false);
    assert!(status["surfaces"].as_array().unwrap().is_empty());
    assert!(!cli(None, &socket, &["native"]).status.success());
    assert!(!cli(None, &socket, &["reload", "--daemon"]).status.success());
}
