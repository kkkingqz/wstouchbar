use futures_lite::future;
use std::{
    io::{self, BufRead, BufReader, Read},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use touchbar_model::Timestamp;
use touchbar_protocol::{DaemonMessage, Mode, RevokedReason};
use touchbard::{
    Command as Action, Daemon, Event, Outgoing,
    config::Config,
    logind::{Monitor, Notice},
};
use zbus::{Connection, zvariant::OwnedObjectPath};
const ROOT: &str = "/org/freedesktop/login1";
const SEAT: &str = "/org/freedesktop/login1/seat/seat0";
fn session_path(index: usize) -> String {
    format!("{ROOT}/session/s{}", index + 1)
}
fn user_path(index: usize) -> String {
    format!("{ROOT}/user/_{}", 1000 + index)
}
fn reference(index: usize) -> (String, OwnedObjectPath) {
    (
        format!("s{}", index + 1),
        session_path(index).try_into().unwrap(),
    )
}
#[derive(Default)]
struct State {
    active: usize,
    locked: [bool; 2],
    greeter: bool,
    sleep: bool,
    reads: usize,
    block: Option<async_channel::Receiver<()>>,
    entered: Option<async_channel::Sender<()>>,
    peers: Vec<std::fs::File>,
}
#[allow(clippy::disallowed_types)]
mod service {
    use super::*;
    pub struct Manager(pub Arc<Mutex<State>>);
    #[zbus::interface(name = "org.freedesktop.login1.Manager")]
    impl Manager {
        #[zbus(property)]
        fn preparing_for_sleep(&self) -> bool {
            self.0.lock().unwrap().sleep
        }
        fn inhibit(
            &self,
            what: &str,
            _who: &str,
            _why: &str,
            mode: &str,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
            if what != "sleep" || mode != "delay" {
                return Err(zbus::fdo::Error::InvalidArgs("expected sleep delay".into()));
            }
            let (r, w) = rustix::pipe::pipe_with(
                rustix::pipe::PipeFlags::CLOEXEC | rustix::pipe::PipeFlags::NONBLOCK,
            )
            .unwrap();
            self.0.lock().unwrap().peers.push(r.into());
            Ok(w.into())
        }
    }
    pub struct Seat(pub Arc<Mutex<State>>);
    #[zbus::interface(name = "org.freedesktop.login1.Seat")]
    impl Seat {
        #[zbus(property)]
        fn active_session(&self) -> (String, OwnedObjectPath) {
            reference(self.0.lock().unwrap().active)
        }
    }
    pub struct Session(pub Arc<Mutex<State>>, pub usize);
    #[zbus::interface(name = "org.freedesktop.login1.Session")]
    impl Session {
        #[zbus(property)]
        fn id(&self) -> String {
            format!("s{}", self.1 + 1)
        }
        #[zbus(property)]
        fn user(&self) -> (u32, OwnedObjectPath) {
            (
                1000 + u32::try_from(self.1).unwrap(),
                user_path(self.1).try_into().unwrap(),
            )
        }
        #[zbus(property)]
        async fn active(&self) -> bool {
            let (active, block, entered) = {
                let mut state = self.0.lock().unwrap();
                state.reads += 1;
                (
                    state.active == self.1,
                    state.block.clone(),
                    state.entered.clone(),
                )
            };
            if let Some(entered) = entered {
                let _ = entered.try_send(());
            }
            if let Some(block) = block {
                let _ = block.recv().await;
            }
            active
        }
        #[zbus(property)]
        fn state(&self) -> &str {
            "active"
        }
        #[zbus(property)]
        fn class(&self) -> &str {
            if self.0.lock().unwrap().greeter {
                "greeter"
            } else {
                "user"
            }
        }
        #[zbus(property, name = "Type")]
        fn kind(&self) -> &str {
            "wayland"
        }
        #[zbus(property)]
        fn locked_hint(&self) -> bool {
            self.0.lock().unwrap().locked[self.1]
        }
    }
    pub struct User(pub usize);
    #[zbus::interface(name = "org.freedesktop.login1.User")]
    impl User {
        #[zbus(property, name = "UID")]
        fn uid(&self) -> u32 {
            1000 + u32::try_from(self.0).unwrap()
        }
        #[zbus(property)]
        fn display(&self) -> (String, OwnedObjectPath) {
            reference(self.0)
        }
    }
}
struct Bus {
    child: Child,
    address: String,
}
impl Bus {
    fn new() -> Self {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Self {
            child,
            address: address.trim().into(),
        }
    }
}
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
struct Server {
    connection: Connection,
    state: Arc<Mutex<State>>,
}
impl Server {
    async fn new(address: &str) -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let mut builder = zbus::connection::Builder::address(address)
            .unwrap()
            .name("org.freedesktop.login1")
            .unwrap()
            .serve_at(ROOT, service::Manager(state.clone()))
            .unwrap()
            .serve_at(SEAT, service::Seat(state.clone()))
            .unwrap();
        for i in 0..2 {
            builder = builder
                .serve_at(session_path(i), service::Session(state.clone(), i))
                .unwrap()
                .serve_at(user_path(i), service::User(i))
                .unwrap();
        }
        Self {
            connection: builder.build().await.unwrap(),
            state,
        }
    }
    async fn changed(&self, path: &str, interface: &str, keys: &[&str]) {
        self.connection
            .emit_signal(
                None::<&str>,
                path,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                &(
                    interface,
                    std::collections::BTreeMap::<String, zbus::zvariant::OwnedValue>::new(),
                    keys,
                ),
            )
            .await
            .unwrap();
    }
    async fn sleep(&self, value: bool) {
        self.state.lock().unwrap().sleep = value;
        self.connection
            .emit_signal(
                None::<&str>,
                ROOT,
                "org.freedesktop.login1.Manager",
                "PrepareForSleep",
                &(value,),
            )
            .await
            .unwrap();
    }
    fn last_closed(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        match state
            .peers
            .last_mut()
            .expect("no inhibitor acquired")
            .read(&mut [0u8; 1])
        {
            Ok(0) => true,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => false,
            r => panic!("unexpected inhibitor state {r:?}"),
        }
    }
}
async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    future::or(future, async {
        async_io::Timer::after(Duration::from_secs(6)).await;
        panic!("private logind timed out");
    })
    .await
}
fn apply(m: &Monitor, d: &mut Daemon, now: u64, event: Event) -> Vec<Action> {
    let cmds = d.handle(Timestamp(now), event);
    for cmd in &cmds {
        match cmd {
            Action::AcquireInhibitor { generation } => m.acquire_inhibitor(*generation),
            Action::ReleaseInhibitor { generation } => m.release_inhibitor(*generation),
            _ => {}
        }
    }
    cmds
}
async fn until(m: &mut Monitor, d: &mut Daemon, now: u64, mut done: impl FnMut(&Event) -> bool) {
    bounded(async {
        loop {
            match m.next().await.unwrap() {
                Notice::Fact(event) => {
                    let end = done(&event);
                    apply(m, d, now, event);
                    if end {
                        return;
                    }
                }
                Notice::Unavailable(_) => {}
            }
        }
    })
    .await;
}
async fn tick(m: &mut Monitor, d: &mut Daemon, now: u64) {
    future::or(
        async {
            loop {
                if let Notice::Fact(event) = m.next().await.unwrap() {
                    apply(m, d, now, event);
                }
            }
        },
        async {
            async_io::Timer::after(Duration::from_millis(30)).await;
        },
    )
    .await;
}
fn drain(d: &mut Daemon, id: u64, now: u64) -> Vec<Outgoing> {
    let mut result = Vec::new();
    while let Some(item) = d.pending(id).cloned() {
        result.push(item);
        d.handle(Timestamp(now), Event::Sent { connection: id });
    }
    result
}
#[test]
#[ignore = "requires dbus-daemon; starts only a private fake logind, never the host service"]
fn env_dbus_logind_lifecycle() {
    if std::env::var_os("WSTOUCHBAR_LOGIND_CHILD").is_none() {
        let bus = Bus::new();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "env_dbus_logind_lifecycle",
                "--nocapture",
            ])
            .env("WSTOUCHBAR_LOGIND_CHILD", "1")
            .env("DBUS_SYSTEM_BUS_ADDRESS", &bus.address)
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    future::block_on(async {
        let address = std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap();
        let server = Server::new(&address).await;
        let mut m = Monitor::new();
        let mut d = Daemon::new(Config::default(), false, Timestamp(0)).unwrap();
        until(
            &mut m,
            &mut d,
            0,
            |e| matches!(e,Event::Session(Some(s)) if s.eligible(1000)),
        )
        .await;
        until(&mut m, &mut d, 0, |e| {
            matches!(e, Event::InhibitorAcquired { .. })
        })
        .await;
        assert!(!server.last_closed());
        apply(&m, &mut d, 0, Event::Device(Some(7)));
        let generation = d.mode().generation();
        apply(&m, &mut d, 0, Event::NativeSafe { generation });
        apply(
            &m,
            &mut d,
            0,
            Event::Hello {
                connection: 1,
                uid: 1000,
                version: 2,
            },
        );
        drain(&mut d, 1, 0);
        d.advance(Timestamp(2000));
        assert_eq!(d.mode().target(), Mode::Display);
        server.state.lock().unwrap().locked[0] = true;
        server
            .changed(
                &session_path(0),
                "org.freedesktop.login1.Session",
                &["LockedHint"],
            )
            .await;
        until(
            &mut m,
            &mut d,
            2100,
            |e| matches!(e,Event::Session(Some(s)) if s.locked),
        )
        .await;
        assert_eq!(d.mode().target(), Mode::Native);
        assert_eq!(d.owner(), Some(1));
        server.state.lock().unwrap().locked[0] = false;
        server
            .changed(
                &session_path(0),
                "org.freedesktop.login1.Session",
                &["LockedHint"],
            )
            .await;
        until(
            &mut m,
            &mut d,
            2200,
            |e| matches!(e,Event::Session(Some(s)) if !s.locked),
        )
        .await;
        assert_eq!(d.mode().target(), Mode::Display);
        server.state.lock().unwrap().active = 1;
        server
            .changed(SEAT, "org.freedesktop.login1.Seat", &["ActiveSession"])
            .await;
        until(
            &mut m,
            &mut d,
            2300,
            |e| matches!(e,Event::Session(Some(s)) if s.uid==1001),
        )
        .await;
        assert_eq!(d.owner(), None);
        assert!(drain(&mut d, 1, 2300).iter().any(|o| matches!(
            o,
            Outgoing::Message(DaemonMessage::Revoked(RevokedReason::SessionInactive))
        )));
        apply(
            &m,
            &mut d,
            2300,
            Event::Hello {
                connection: 2,
                uid: 1001,
                version: 2,
            },
        );
        assert_eq!(d.owner(), Some(2));
        server.state.lock().unwrap().locked[0] = true;
        server
            .changed(
                &session_path(0),
                "org.freedesktop.login1.Session",
                &["LockedHint"],
            )
            .await;
        tick(&mut m, &mut d, 2350).await;
        assert_eq!(
            d.mode().target(),
            Mode::Display,
            "old session subscription remained active"
        );
        assert_eq!(d.owner(), Some(2));
        server.state.lock().unwrap().locked[0] = false;
        server.state.lock().unwrap().greeter = true;
        server
            .changed(
                &session_path(1),
                "org.freedesktop.login1.Session",
                &["Class"],
            )
            .await;
        until(
            &mut m,
            &mut d,
            2400,
            |e| matches!(e,Event::Session(Some(s)) if s.class=="greeter"),
        )
        .await;
        apply(
            &m,
            &mut d,
            2400,
            Event::Hello {
                connection: 2,
                uid: 1001,
                version: 2,
            },
        );
        assert_eq!(d.owner(), None);
        // Re-enter a display transition to test that the FD survives the unsafe interval.
        {
            let mut s = server.state.lock().unwrap();
            s.greeter = false;
            s.active = 0;
        }
        server
            .changed(SEAT, "org.freedesktop.login1.Seat", &["ActiveSession"])
            .await;
        until(
            &mut m,
            &mut d,
            2500,
            |e| matches!(e,Event::Session(Some(s)) if s.eligible(1000)),
        )
        .await;
        apply(
            &m,
            &mut d,
            2500,
            Event::Hello {
                connection: 3,
                uid: 1000,
                version: 2,
            },
        );
        d.advance(Timestamp(5000));
        server.sleep(true).await;
        until(&mut m, &mut d, 5100, |e| {
            matches!(e, Event::PrepareForSleep(true))
        })
        .await;
        assert!(!server.last_closed());
        let generation = d.mode().generation();
        apply(&m, &mut d, 5200, Event::NativeSafe { generation });
        tick(&mut m, &mut d, 5200).await;
        assert!(server.last_closed());
        // A fresh process starts during sleep and must not acquire a delay inhibitor.
        drop(m);
        let before = server.state.lock().unwrap().peers.len();
        let mut m = Monitor::new();
        let mut d = Daemon::new(Config::default(), false, Timestamp(0)).unwrap();
        until(&mut m, &mut d, 0, |e| {
            matches!(
                e,
                Event::Start {
                    preparing_sleep: true
                }
            )
        })
        .await;
        tick(&mut m, &mut d, 0).await;
        assert_eq!(server.state.lock().unwrap().peers.len(), before);
        assert!(d.sleeping());
        server.sleep(false).await;
        until(&mut m, &mut d, 100, |e| {
            matches!(e, Event::InhibitorAcquired { .. })
        })
        .await;
        assert!(!server.last_closed());
        // Without NativeSafe the four-second model deadline still closes the real FD.
        apply(&m, &mut d, 100, Event::Device(Some(8)));
        server.sleep(true).await;
        until(&mut m, &mut d, 200, |e| {
            matches!(e, Event::PrepareForSleep(true))
        })
        .await;
        assert!(!server.last_closed());
        let commands = d.advance(Timestamp(4200));
        for command in commands {
            if let Action::ReleaseInhibitor { generation } = command {
                m.release_inhibitor(generation);
            }
        }
        tick(&mut m, &mut d, 4200).await;
        assert!(server.last_closed());
    });
}
#[test]
#[ignore = "requires dbus-daemon; private logind name and inhibitor failure recovery"]
fn env_dbus_logind_owner_loss_keeps_valid_fd_then_detects_pipe_hup() {
    future::block_on(async {
        let bus = Bus::new();
        let server = Server::new(&bus.address).await;
        let mut m = Monitor::with_address(bus.address.clone());
        let mut d = Daemon::new(Config::default(), false, Timestamp(0)).unwrap();
        until(&mut m, &mut d, 0, |e| {
            matches!(e, Event::InhibitorAcquired { .. })
        })
        .await;
        server
            .connection
            .release_name("org.freedesktop.login1")
            .await
            .unwrap();
        until(&mut m, &mut d, 10, |e| matches!(e, Event::Session(None))).await;
        tick(&mut m, &mut d, 10).await;
        assert!(
            !server.last_closed(),
            "D-Bus loss must not close a still-valid delay FD"
        );
        server.state.lock().unwrap().peers.clear();
        until(&mut m, &mut d, 20, |e| {
            matches!(e, Event::InhibitorFailed { .. })
        })
        .await;
        assert!(d.inhibitor_error().is_some());
        server
            .connection
            .request_name("org.freedesktop.login1")
            .await
            .unwrap();
        until(&mut m, &mut d, 30, |e| {
            matches!(e, Event::InhibitorAcquired { .. })
        })
        .await;
        assert_eq!(d.inhibitor_error(), None);
        assert!(!server.last_closed());
    });
}
#[test]
#[ignore = "requires dbus-daemon; kills only its own private bus during a fake sleep"]
fn env_dbus_logind_bus_disconnect_preserves_delay_until_native_safe() {
    future::block_on(async {
        let mut bus = Bus::new();
        let server = Server::new(&bus.address).await;
        let mut m = Monitor::with_address(bus.address.clone());
        let mut d = Daemon::new(Config::default(), false, Timestamp(0)).unwrap();
        until(&mut m, &mut d, 0, |e| {
            matches!(e, Event::InhibitorAcquired { .. })
        })
        .await;
        apply(&m, &mut d, 0, Event::Device(Some(7)));
        server.sleep(true).await;
        until(&mut m, &mut d, 100, |e| {
            matches!(e, Event::PrepareForSleep(true))
        })
        .await;
        assert!(!server.last_closed());
        bus.child.kill().unwrap();
        bus.child.wait().unwrap();
        until(&mut m, &mut d, 200, |e| matches!(e, Event::Session(None))).await;
        assert!(!server.last_closed());
        let generation = d.mode().generation();
        apply(&m, &mut d, 300, Event::NativeSafe { generation });
        tick(&mut m, &mut d, 300).await;
        assert!(server.last_closed());
    });
}
#[test]
#[ignore = "requires dbus-daemon; verifies unrelated logind traffic does not trigger IO"]
fn env_dbus_logind_ignores_unrelated_properties() {
    future::block_on(async {
        let bus = Bus::new();
        let server = Server::new(&bus.address).await;
        let mut m = Monitor::with_address(bus.address.clone());
        let mut d = Daemon::new(Config::default(), false, Timestamp(0)).unwrap();
        until(&mut m, &mut d, 0, |e| {
            matches!(e, Event::InhibitorAcquired { .. })
        })
        .await;
        let before = server.state.lock().unwrap().reads;
        server
            .changed(
                &session_path(0),
                "org.freedesktop.login1.Session",
                &["IdleHint"],
            )
            .await;
        tick(&mut m, &mut d, 0).await;
        assert_eq!(
            server.state.lock().unwrap().reads,
            before,
            "unrelated property caused another GetAll"
        );
    });
}
#[test]
#[ignore = "requires dbus-daemon; release must not wait for a blocked logind property"]
fn env_dbus_logind_release_remains_responsive_during_snapshot() {
    future::block_on(async {
        let bus = Bus::new();
        let server = Server::new(&bus.address).await;
        let mut m = Monitor::with_address(bus.address.clone());
        let mut d = Daemon::new(Config::default(), false, Timestamp(0)).unwrap();
        let mut generation = 0;
        until(&mut m, &mut d, 0, |e| {
            if let Event::InhibitorAcquired { generation: g } = e {
                generation = *g;
                true
            } else {
                false
            }
        })
        .await;
        let (unblock, block) = async_channel::bounded(1);
        let (entered, enter) = async_channel::bounded(1);
        {
            let mut state = server.state.lock().unwrap();
            state.block = Some(block);
            state.entered = Some(entered);
        }
        server
            .changed(
                &session_path(0),
                "org.freedesktop.login1.Session",
                &["LockedHint"],
            )
            .await;
        bounded(future::or(
            async {
                enter.recv().await.unwrap();
            },
            async {
                loop {
                    if let Notice::Fact(event) = m.next().await.unwrap() {
                        apply(&m, &mut d, 0, event);
                    }
                }
            },
        ))
        .await;
        let fd = server
            .state
            .lock()
            .unwrap()
            .peers
            .last()
            .unwrap()
            .try_clone()
            .unwrap();
        let fd = async_io::Async::new(fd).unwrap();
        m.release_inhibitor(generation);
        future::or(
            async {
                loop {
                    let _ = m.next().await.unwrap();
                }
            },
            future::or(
                async {
                    let n = fd
                        .read_with(|fd| {
                            rustix::io::read(fd, &mut [0u8; 1]).map_err(io::Error::from)
                        })
                        .await
                        .unwrap();
                    assert_eq!(n, 0);
                },
                async {
                    async_io::Timer::after(Duration::from_millis(200)).await;
                    panic!("release waited for a blocked GetAll");
                },
            ),
        )
        .await;
        unblock.send(()).await.unwrap();
    });
}
#[test]
#[ignore = "requires dbus-daemon; startup must account for a sleep signal during its snapshot"]
fn env_dbus_logind_initial_sleep_signal_wins_over_older_property() {
    future::block_on(async {
        let bus = Bus::new();
        let server = Server::new(&bus.address).await;
        let (unblock, block) = async_channel::bounded(1);
        let (entered, enter) = async_channel::bounded(1);
        {
            let mut state = server.state.lock().unwrap();
            state.block = Some(block);
            state.entered = Some(entered);
        }
        let mut m = Monitor::with_address(bus.address.clone());
        let mut d = Daemon::new(Config::default(), false, Timestamp(0)).unwrap();
        bounded(future::or(
            async {
                enter.recv().await.unwrap();
            },
            async {
                loop {
                    let _ = m.next().await.unwrap();
                }
            },
        ))
        .await;
        server.sleep(true).await;
        unblock.send(()).await.unwrap();
        until(&mut m, &mut d, 0, |e| {
            matches!(
                e,
                Event::Start {
                    preparing_sleep: true
                }
            )
        })
        .await;
        tick(&mut m, &mut d, 0).await;
        assert!(
            server.state.lock().unwrap().peers.is_empty(),
            "startup acquired during sleep"
        );
        assert!(d.sleeping());
    });
}
