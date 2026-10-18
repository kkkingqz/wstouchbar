use std::{
    collections::{BTreeSet, VecDeque},
    io,
    os::fd::OwnedFd,
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};
use touchbar_model::WatchKey;
use touchbar_protocol::{self as wire, DaemonMessage, DaemonPacket, SessionMessage, control};
use touchbard::{
    Command, Event, Session,
    config::Config,
    runtime::{Drivers, Runtime},
};
struct Fake {
    events: VecDeque<Event>,
    incoming: async_channel::Receiver<Vec<Event>>,
    shift_reported: bool,
    cleaned: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Drivers for Fake {
    fn dispatch(&mut self, command: Command) -> Vec<Event> {
        match command {
            Command::AcquireInhibitor { generation } => {
                vec![Event::InhibitorAcquired { generation }]
            }
            _ => Vec::new(),
        }
    }
    fn watch_keys(&mut self, wanted: BTreeSet<WatchKey>) -> Vec<Event> {
        let held = wanted.contains(&WatchKey::RightShift);
        if held == self.shift_reported {
            return Vec::new();
        }
        self.shift_reported = held;
        vec![Event::PhysicalKey(touchbar_bindings::KeyInput {
            code: WatchKey::RightShift,
            phase: if held {
                touchbar_model::KeyPhase::Pressed
            } else {
                touchbar_model::KeyPhase::Released
            },
        })]
    }
    fn buffers(&self, _: u64) -> Option<&[OwnedFd]> {
        None
    }
    fn reload(&mut self) -> io::Result<Config> {
        Ok(Config::default())
    }
    fn status(&self) -> serde_json::Value {
        serde_json::json!({"fake":true})
    }
    async fn finish(&mut self, _: std::time::Instant) -> io::Result<()> {
        async_io::Timer::after(Duration::from_millis(10)).await;
        self.cleaned
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    async fn next(&mut self) -> io::Result<Vec<Event>> {
        if !self.events.is_empty() {
            return Ok(self.events.drain(..).collect());
        }
        self.incoming
            .recv()
            .await
            .map_err(|_| io::ErrorKind::UnexpectedEof.into())
    }
}
fn session(uid: u32) -> Session {
    Session {
        uid,
        id: "graphical".into(),
        user_display: Some("graphical".into()),
        active: true,
        state: "active".into(),
        class: "user".into(),
        kind: "wayland".into(),
        locked: false,
    }
}
fn fake(uid: u32) -> (Fake, async_channel::Sender<Vec<Event>>) {
    let (tx, rx) = async_channel::bounded(8);
    (
        Fake {
            events: VecDeque::from([
                Event::Start {
                    preparing_sleep: false,
                },
                Event::Session(Some(session(uid))),
                Event::Device(None),
            ]),
            incoming: rx,
            cleaned: Default::default(),
            shift_reported: false,
        },
        tx,
    )
}
fn connect(path: &Path) -> OwnedFd {
    use rustix::net::*;
    let fd = socket(AddressFamily::UNIX, SocketType::SEQPACKET, None).unwrap();
    rustix::net::connect(&fd, &SocketAddrUnix::new(path).unwrap()).unwrap();
    fd
}
fn control(path: &Path, command: control::Command) -> control::Reply {
    use std::io::Write;
    let mut fd = UnixStream::connect(path).unwrap();
    fd.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    fd.write_all(&control::encode(&control::Request::new(command)).unwrap())
        .unwrap();
    let r: control::Response = control::read_frame(&mut fd).unwrap();
    r.reply
}
#[test]
fn real_sockets_hello_status_setmode_and_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let uid = rustix::process::getuid().as_raw();
    let (drivers, tx) = fake(uid);
    let cleaned = drivers.cleaned.clone();
    let mut runtime = Runtime::new(Config::default(), false, dir.path(), drivers).unwrap();
    let path = dir.path().to_owned();
    let client = std::thread::spawn(move || {
        // Ask status first so startup facts have been consumed before Hello.
        let status = control(&path.join("control.sock"), control::Command::Status);
        assert!(
            matches!(status,control::Reply::Status(ref v) if v["owner"]["uid"]==uid && v["device"]["state"]["Native"]["safe"].is_boolean())
        );
        let fd = connect(&path.join("session.sock"));
        wire::send_session(&fd, SessionMessage::Hello { version: 2 }).unwrap();
        assert!(matches!(
            wire::recv_daemon(&fd).unwrap(),
            DaemonPacket::Message(DaemonMessage::ModeChanged {
                mode: wire::Mode::Native,
                ..
            })
        ));
        let reload = control(&path.join("control.sock"), control::Command::Reload);
        if uid == 0 {
            assert_eq!(reload, control::Reply::Ack);
        } else {
            assert!(matches!(reload, control::Reply::Error(_)));
        }
        assert_eq!(
            control(
                &path.join("control.sock"),
                control::Command::SetMode {
                    mode: wire::Mode::Native
                }
            ),
            control::Reply::Ack
        );
        assert!(
            matches!(control(&path.join("control.sock"),control::Command::Status),control::Reply::Status(ref v) if v["recovery"]==true)
        );
        tx.send_blocking(vec![Event::Shutdown]).unwrap();
        // Revocation is flushed before the runtime exits.
        assert!(matches!(
            wire::recv_daemon(&fd).unwrap(),
            DaemonPacket::Message(DaemonMessage::Revoked { .. })
        ));
    });
    futures_lite::future::block_on(runtime.run()).unwrap();
    client.join().unwrap();
    assert!(
        cleaned.load(std::sync::atomic::Ordering::SeqCst),
        "pending cleanup was discarded at Exit"
    );
    drop(runtime);
    assert!(!dir.path().join("session.sock").exists());
}
#[test]
fn foreign_session_uid_is_rejected_using_real_peercred() {
    let dir = tempfile::tempdir().unwrap();
    let uid = rustix::process::getuid().as_raw();
    let (drivers, tx) = fake(uid + 1);
    let mut runtime = Runtime::new(Config::default(), false, dir.path(), drivers).unwrap();
    let path = dir.path().to_owned();
    let client = std::thread::spawn(move || {
        let _ = control(&path.join("control.sock"), control::Command::Status);
        let fd = connect(&path.join("session.sock"));
        wire::send_session(&fd, SessionMessage::Hello { version: 2 }).unwrap();
        assert!(matches!(
            wire::recv_daemon(&fd).unwrap(),
            DaemonPacket::Message(DaemonMessage::Revoked(wire::RevokedReason::NotActive))
        ));
        assert!(wire::recv_daemon(&fd).is_err());
        tx.send_blocking(vec![Event::Shutdown]).unwrap();
    });
    futures_lite::future::block_on(runtime.run()).unwrap();
    client.join().unwrap();
}
#[test]
fn sigterm_closes_runtime_in_a_separate_process() {
    const ENV: &str = "WSTOUCHBAR_RUNTIME_SIGNAL_TEST";
    if let Ok(path) = std::env::var(ENV) {
        let (drivers, _tx) = fake(rustix::process::getuid().as_raw());
        let mut runtime =
            Runtime::new(Config::default(), false, Path::new(&path), drivers).unwrap();
        futures_lite::future::block_on(runtime.run()).unwrap();
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "sigterm_closes_runtime_in_a_separate_process",
            "--nocapture",
        ])
        .env(ENV, dir.path())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    while !dir.path().join("control.sock").exists() {
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(
        control(&dir.path().join("control.sock"), control::Command::Status),
        control::Reply::Status(_)
    ));
    let result = std::process::Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(result.success());
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            child.kill().unwrap();
            panic!("SIGTERM did not finish");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!dir.path().join("session.sock").exists());
}

#[test]
fn enabling_watch_reports_a_key_already_held_before_subscription() {
    let dir = tempfile::tempdir().unwrap();
    let (drivers, tx) = fake(rustix::process::getuid().as_raw());
    let mut runtime = Runtime::new(Config::default(), false, dir.path(), drivers).unwrap();
    let path = dir.path().to_owned();
    let client = std::thread::spawn(move || {
        let _ = control(&path.join("control.sock"), control::Command::Status);
        let fd = connect(&path.join("session.sock"));
        wire::send_session(&fd, SessionMessage::Hello { version: 2 }).unwrap();
        let _ = wire::recv_daemon(&fd).unwrap();
        wire::send_session(
            &fd,
            SessionMessage::WatchKey {
                key: WatchKey::RightShift,
                enabled: true,
            },
        )
        .unwrap();
        assert!(matches!(
            wire::recv_daemon(&fd).unwrap(),
            DaemonPacket::Message(DaemonMessage::KeyEvent {
                key: WatchKey::RightShift,
                phase: touchbar_model::KeyPhase::Pressed
            })
        ));
        tx.send_blocking(vec![Event::Shutdown]).unwrap();
    });
    futures_lite::future::block_on(runtime.run()).unwrap();
    client.join().unwrap();
}

#[test]
fn saturated_client_is_disconnected_without_blocking_control_or_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let (drivers, tx) = fake(rustix::process::getuid().as_raw());
    let mut runtime = Runtime::new(Config::default(), false, dir.path(), drivers).unwrap();
    let path = dir.path().to_owned();
    let client = std::thread::spawn(move || {
        let _ = control(&path.join("control.sock"), control::Command::Status);
        let fd = connect(&path.join("session.sock"));
        wire::send_session(&fd, SessionMessage::Hello { version: 2 }).unwrap();
        let _ = wire::recv_daemon(&fd).unwrap();
        let mut rejected = false;
        for i in 0..4000 {
            if wire::send_session(
                &fd,
                SessionMessage::WatchKey {
                    key: WatchKey::Esc,
                    enabled: i % 2 == 0,
                },
            )
            .is_err()
            {
                rejected = true;
                break;
            }
        }
        let _ = rejected; // Close is allowed a terminal-message drain interval.
        assert!(
            matches!(control(&path.join("control.sock"),control::Command::Status),control::Reply::Status(ref v) if v["connection"].is_null())
        );
        let mut drained = 0;
        while wire::recv_daemon(&fd).is_ok() {
            drained += 1;
            assert!(drained < 4000, "unbounded egress queue");
        }
        tx.send_blocking(vec![Event::Shutdown]).unwrap();
    });
    futures_lite::future::block_on(async {
        futures_lite::future::or(runtime.run(), async {
            async_io::Timer::after(Duration::from_secs(5)).await;
            panic!("full client blocked loop")
        })
        .await
        .unwrap();
    });
    client.join().unwrap();
}
