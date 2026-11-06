//! Full shell and real sockets; all device/logind effects terminate at RecordingDrivers.
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use touchbar_model::{KeyPhase, SystemKey, TouchPhase, WatchKey};
use touchbar_protocol::{
    self as wire, DaemonMessage as Out, DaemonPacket, Mode, ModeReason, SessionMessage as In,
};
use touchbar_testkit::TestSessionPeer;
use touchbard::{
    Command, DisplayInfo, Event, Session,
    config::Config,
    machine::mode,
    runtime::{Drivers, Runtime, marker::Recovery},
};
const LIMIT: Duration = Duration::from_secs(12);
type Log = Arc<Mutex<Vec<(Instant, Command)>>>;
fn graphical(uid: u32) -> Session {
    Session {
        uid,
        id: "test-graphics".into(),
        user_display: Some("test-graphics".into()),
        active: true,
        state: "active".into(),
        class: "user".into(),
        kind: "wayland".into(),
        locked: false,
    }
}
fn info() -> DisplayInfo {
    DisplayInfo {
        header: wire::SwapchainHeader {
            logical_width: 20,
            logical_height: 4,
            physical_width: 20,
            physical_height: 4,
            pitch: 80,
            buffer_size: 320,
            buffer_count: 3,
            frame_damage: true,
            format: wire::XRGB8888,
        },
        caps: 1,
        width_mm100: 2000,
        height_mm100: 400,
    }
}
struct RecordingDrivers {
    initial: VecDeque<Event>,
    incoming: async_channel::Receiver<Vec<Event>>,
    buffers: BTreeMap<u64, Vec<OwnedFd>>,
    log: Log,
    run: PathBuf,
}
impl Drivers for RecordingDrivers {
    fn dispatch(&mut self, c: Command) -> Vec<Event> {
        self.log.lock().unwrap().push((Instant::now(), c.clone()));
        match c {
            Command::Device(mode::Command::ToNative { generation, .. }) => {
                vec![Event::NativeSafe { generation }]
            }
            Command::Device(
                mode::Command::ToDisplay { generation, .. }
                | mode::Command::RefreshSwapchain { generation, .. },
            ) => {
                let mut buffers = Vec::new();
                for _ in 0..3 {
                    let fd = rustix::fs::memfd_create(
                        "daemon-e2e",
                        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
                    )
                    .unwrap();
                    rustix::fs::ftruncate(&fd, 320).unwrap();
                    rustix::fs::fcntl_add_seals(
                        &fd,
                        rustix::fs::SealFlags::SHRINK
                            | rustix::fs::SealFlags::GROW
                            | rustix::fs::SealFlags::SEAL,
                    )
                    .unwrap();
                    buffers.push(fd);
                }
                self.buffers.insert(generation, buffers);
                vec![Event::DisplayReady {
                    generation,
                    info: info(),
                }]
            }
            Command::AcquireInhibitor { generation } => {
                vec![Event::InhibitorAcquired { generation }]
            }
            Command::WriteRecovery(value) => {
                Recovery::new(&self.run).write(value).unwrap();
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
    fn watch_keys(&mut self, _: BTreeSet<WatchKey>) -> Vec<Event> {
        Vec::new()
    }
    fn buffers(&self, generation: u64) -> Option<&[OwnedFd]> {
        self.buffers.get(&generation).map(Vec::as_slice)
    }
    fn reload(&mut self) -> io::Result<Config> {
        Ok(Config::default())
    }
    fn status(&self) -> serde_json::Value {
        serde_json::json!({"fake":true})
    }
    async fn next(&mut self) -> io::Result<Vec<Event>> {
        if !self.initial.is_empty() {
            return Ok(self.initial.drain(..).collect());
        }
        self.incoming
            .recv()
            .await
            .map_err(|_| io::ErrorKind::UnexpectedEof.into())
    }
    async fn finish(&mut self, _: Instant) -> io::Result<()> {
        Ok(())
    }
}
fn commands(log: &Log) -> Vec<Command> {
    log.lock().unwrap().iter().map(|(_, c)| c.clone()).collect()
}
async fn until(log: &Log, predicate: impl Fn(&[Command]) -> bool) {
    let start = Instant::now();
    loop {
        let c = commands(log);
        if predicate(&c) {
            return;
        }
        assert!(start.elapsed() < LIMIT, "commands: {c:?}");
        async_io::Timer::after(Duration::from_millis(5)).await;
    }
}
async fn mode(peer: &TestSessionPeer, wanted: Mode, reason: Option<ModeReason>) {
    loop {
        match peer.receive(LIMIT).await.unwrap() {
            DaemonPacket::Message(Out::ModeChanged { mode, reason: r }) if mode == wanted => {
                if let Some(reason) = reason {
                    assert_eq!(r, reason)
                }
                return;
            }
            DaemonPacket::Message(Out::Revoked(r)) => panic!("unexpected revocation {r:?}"),
            _ => {}
        }
    }
}
async fn display(peer: &TestSessionPeer) {
    let mut welcome = false;
    let mut changed = false;
    loop {
        match peer.receive(LIMIT).await.unwrap() {
            DaemonPacket::Message(Out::Welcome { width, height, .. }) => {
                assert_eq!((width, height), (20, 4));
                welcome = true;
            }
            DaemonPacket::Message(Out::ModeChanged {
                mode: Mode::Display,
                ..
            }) => {
                assert!(welcome);
                changed = true
            }
            DaemonPacket::Swapchain { header, buffers } => {
                assert!(welcome && changed);
                assert_eq!(header, info().header);
                assert_eq!(buffers.len(), 3);
                return;
            }
            DaemonPacket::Message(Out::Revoked(r)) => panic!("unexpected revocation {r:?}"),
            _ => {}
        }
    }
}
fn key(code: WatchKey, phase: KeyPhase) -> Event {
    Event::PhysicalKey(touchbar_bindings::KeyInput { code, phase })
}
#[derive(Clone, Copy, Debug)]
enum Case {
    Login,
    Switch,
    Restart,
    Stop,
    Recovery,
    Persist,
    Restored,
    Sleep,
    Lock,
    Wake,
    FnNative,
}
async fn exercise(dir: &Path, case: Case) {
    let uid = rustix::process::getuid().as_raw();
    let active = graphical(uid);
    let (tx, rx) = async_channel::bounded(32);
    let log: Log = Default::default();
    let mut config = Config::default();
    if matches!(case, Case::Wake) {
        config.dim_timeout = Some(1);
        config.idle_timeout = Some(1);
    } else {
        config.dim_timeout = Some(60);
        config.idle_timeout = Some(60);
    }
    let drivers = RecordingDrivers {
        initial: VecDeque::from([
            Event::Start {
                preparing_sleep: false,
            },
            Event::Session(Some(active.clone())),
            Event::Device(Some(7)),
            Event::Driver {
                device: 7,
                mode: Some(1),
            },
            Event::Autodim(true),
        ]),
        incoming: rx,
        buffers: BTreeMap::new(),
        log: log.clone(),
        run: dir.into(),
    };
    let mut runtime =
        Runtime::new(config, Recovery::new(dir).read().unwrap(), dir, drivers).unwrap();
    let scenario = async {
        until(&log, |c| {
            c.iter()
                .any(|x| matches!(x, Command::AcquireInhibitor { .. }))
        })
        .await;
        let peer = TestSessionPeer::connect(&dir.join("session.sock")).unwrap();
        peer.send(In::Hello {
            version: wire::VERSION,
        })
        .unwrap();
        mode(&peer, Mode::Native, None).await;
        if !matches!(case, Case::FnNative | Case::Restored) {
            display(&peer).await;
        }
        let start = commands(&log).len();
        match case {
            Case::Login => {
                assert!(
                    commands(&log)
                        .iter()
                        .any(|x| matches!(x, Command::Device(mode::Command::ToDisplay { .. })))
                );
            }
            Case::Switch => {
                peer.send(In::Key {
                    key: SystemKey::F5,
                    phase: KeyPhase::Pressed,
                })
                .unwrap();
                until(&log, |c| {
                    c.contains(&Command::Uinput {
                        key: SystemKey::F5,
                        phase: KeyPhase::Pressed,
                    })
                })
                .await;
                tx.send(vec![Event::Touch {
                    tracking: 19,
                    phase: TouchPhase::Down,
                    x: 2,
                    y: 2,
                }])
                .await
                .unwrap();
                assert!(matches!(
                    peer.receive(LIMIT).await.unwrap(),
                    DaemonPacket::Message(Out::Touch {
                        phase: TouchPhase::Down,
                        ..
                    })
                ));
                let at = Instant::now();
                tx.send(vec![Event::Session(Some(graphical(uid + 1)))])
                    .await
                    .unwrap();
                let mut cancelled = false;
                loop {
                    match peer.receive(LIMIT).await.unwrap() {
                        DaemonPacket::Message(Out::Touch {
                            phase: TouchPhase::Cancel,
                            ..
                        }) => cancelled = true,
                        DaemonPacket::Message(Out::Revoked(
                            wire::RevokedReason::SessionInactive,
                        )) => break,
                        other => panic!("unexpected switch packet {other:?}"),
                    }
                }
                assert!(cancelled);
                assert!(at.elapsed() < Duration::from_secs(1));
                assert!(peer.receive(LIMIT).await.is_err());
                until(&log, |c| {
                    c[start..]
                        .iter()
                        .any(|c| matches!(c, Command::Device(mode::Command::ToNative { .. })))
                })
                .await;
                let c = commands(&log);
                let release = c
                    .iter()
                    .rposition(|c| {
                        *c == Command::Uinput {
                            key: SystemKey::F5,
                            phase: KeyPhase::Released,
                        }
                    })
                    .unwrap();
                let native = c
                    .iter()
                    .rposition(|c| matches!(c, Command::Device(mode::Command::ToNative { .. })))
                    .unwrap();
                assert!(release < native);
            }
            Case::Restart => {
                drop(peer);
                async_io::Timer::after(Duration::from_millis(100)).await;
                let peer = TestSessionPeer::connect(&dir.join("session.sock")).unwrap();
                peer.send(In::Hello {
                    version: wire::VERSION,
                })
                .unwrap();
                display(&peer).await;
                assert!(!commands(&log)[start..].iter().any(|c| matches!(
                    c,
                    Command::Device(
                        mode::Command::ToNative { .. } | mode::Command::ToDisplay { .. }
                    )
                )));
            }
            Case::Stop => {
                let at = Instant::now();
                drop(peer);
                until(&log, |c| {
                    c[start..]
                        .iter()
                        .any(|c| matches!(c, Command::Device(mode::Command::ToNative { .. })))
                })
                .await;
                assert!(at.elapsed() >= Duration::from_millis(1900));
                assert!(at.elapsed() < Duration::from_secs(3));
            }
            Case::Recovery => {
                tx.send(vec![key(WatchKey::Fn, KeyPhase::Pressed)])
                    .await
                    .unwrap();
                async_io::Timer::after(Duration::from_millis(7900)).await;
                assert!(!commands(&log)[start..].contains(&Command::WriteRecovery(true)));
                mode(&peer, Mode::Native, Some(ModeReason::Recovery)).await;
                assert!(Recovery::new(dir).read().unwrap());
                tx.send(vec![
                    key(WatchKey::Fn, KeyPhase::Released),
                    key(WatchKey::Fn, KeyPhase::Pressed),
                ])
                .await
                .unwrap();
                display(&peer).await;
                assert!(!Recovery::new(dir).read().unwrap());
            }
            Case::Persist => {
                tx.send(vec![Event::SetMode(Mode::Native)]).await.unwrap();
                mode(&peer, Mode::Native, Some(ModeReason::Recovery)).await;
                until(&log, |c| c.contains(&Command::WriteRecovery(true))).await;
                assert!(Recovery::new(dir).read().unwrap());
            }
            Case::Restored => {
                assert!(Recovery::new(dir).read().unwrap());
                assert!(
                    !commands(&log)
                        .iter()
                        .any(|c| matches!(c, Command::Device(mode::Command::ToDisplay { .. })))
                );
                tx.send(vec![Event::SetMode(Mode::Display)]).await.unwrap();
                display(&peer).await;
                assert!(!Recovery::new(dir).read().unwrap());
            }
            Case::Sleep => {
                tx.send(vec![Event::PrepareForSleep(true)]).await.unwrap();
                mode(&peer, Mode::Native, Some(ModeReason::Sleep)).await;
                until(&log, |c| {
                    c[start..]
                        .iter()
                        .any(|c| matches!(c, Command::ReleaseInhibitor { .. }))
                })
                .await;
                let c = commands(&log);
                let native = c[start..]
                    .iter()
                    .position(|c| matches!(c, Command::Device(mode::Command::ToNative { .. })))
                    .unwrap();
                let release = c[start..]
                    .iter()
                    .position(|c| matches!(c, Command::ReleaseInhibitor { .. }))
                    .unwrap();
                assert!(native < release);
                tx.send(vec![Event::PrepareForSleep(false)]).await.unwrap();
                display(&peer).await;
            }
            Case::Lock => {
                let mut locked = active.clone();
                locked.locked = true;
                tx.send(vec![Event::Session(Some(locked))]).await.unwrap();
                mode(&peer, Mode::Native, Some(ModeReason::Locked)).await;
                tx.send(vec![Event::Session(Some(active))]).await.unwrap();
                display(&peer).await;
            }
            Case::Wake => {
                until(&log, |c| {
                    c.iter().rev().find_map(|c| match c {
                        Command::SetBacklight(level) => Some(*level),
                        _ => None,
                    }) == Some(0)
                })
                .await;
                tx.send(vec![
                    Event::Touch {
                        tracking: 1,
                        phase: TouchPhase::Down,
                        x: 2,
                        y: 2,
                    },
                    Event::Touch {
                        tracking: 1,
                        phase: TouchPhase::Move,
                        x: 3,
                        y: 2,
                    },
                    Event::Touch {
                        tracking: 1,
                        phase: TouchPhase::Up,
                        x: 3,
                        y: 2,
                    },
                ])
                .await
                .unwrap();
                until(&log, |c| c[start..].contains(&Command::SetBacklight(2))).await;
                assert_eq!(
                    peer.receive(Duration::from_millis(30))
                        .await
                        .unwrap_err()
                        .kind(),
                    io::ErrorKind::TimedOut
                );
                tx.send(vec![Event::Touch {
                    tracking: 2,
                    phase: TouchPhase::Down,
                    x: 2,
                    y: 2,
                }])
                .await
                .unwrap();
                assert!(matches!(
                    peer.receive(LIMIT).await.unwrap(),
                    DaemonPacket::Message(Out::Touch {
                        phase: TouchPhase::Down,
                        ..
                    })
                ));
            }
            Case::FnNative => {
                tx.send(vec![key(WatchKey::Fn, KeyPhase::Pressed)])
                    .await
                    .unwrap();
                until(&log, |c| c[start..].contains(&Command::WriteDriverMode(2))).await;
                tx.send(vec![key(WatchKey::Fn, KeyPhase::Released)])
                    .await
                    .unwrap();
                until(&log, |c| c[start..].contains(&Command::WriteDriverMode(1))).await;
                assert!(
                    !commands(&log)
                        .iter()
                        .any(|c| matches!(c, Command::Device(mode::Command::ToDisplay { .. })))
                );
            }
        }
        tx.send(vec![Event::Shutdown]).await.unwrap();
    };
    futures_lite::future::zip(runtime.run(), scenario)
        .await
        .0
        .unwrap();
}
macro_rules! cases {($($name:ident=>$case:ident),* $(,)?)=>{$(#[test] fn $name(){let dir=tempfile::tempdir().unwrap();futures_lite::future::block_on(exercise(dir.path(),Case::$case));})*};}
cases! {login_switches_to_display=>Login,fast_user_switch_revokes=>Switch,session_restart_within_grace=>Restart,session_stop_goes_native=>Stop,recovery_hold_fn=>Recovery,sleep_goes_native_first=>Sleep,lock_screen_native=>Lock,wake_touch_not_delivered=>Wake}
#[test]
fn recovery_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    futures_lite::future::block_on(async {
        exercise(dir.path(), Case::Persist).await;
        exercise(dir.path(), Case::Restored).await;
    });
}
#[test]
fn fn_bridge_in_native() {
    let dir = tempfile::tempdir().unwrap();
    Recovery::new(dir.path()).write(true).unwrap();
    futures_lite::future::block_on(exercise(dir.path(), Case::FnNative));
}
