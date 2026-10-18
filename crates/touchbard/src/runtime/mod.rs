//! Linux shell around the deterministic daemon policy.
pub mod sockets;
use crate::{Command, Daemon, Event, Outgoing, config::Config};
use futures_lite::{
    StreamExt,
    future::{or, pending},
};
use sockets::{Incoming, Sockets};
use std::{
    collections::{BTreeSet, VecDeque},
    io,
    os::fd::OwnedFd,
    path::Path,
    time::{Duration, Instant},
};
use touchbar_model::{Timestamp, WatchKey};
use touchbar_protocol::{SessionMessage, control};
/// IO boundary used by the real Linux drivers and in-process daemon tests.
#[allow(async_fn_in_trait)]
pub trait Drivers {
    fn dispatch(&mut self, command: Command) -> Vec<Event>;
    /// Refresh authoritative physical state before enabling a watch or replacing bindings.
    fn watch_keys(&mut self, wanted: BTreeSet<WatchKey>) -> Vec<Event>;
    fn buffers(&self, generation: u64) -> Option<&[OwnedFd]>;
    fn reload(&mut self) -> io::Result<Config>;
    fn status(&self) -> serde_json::Value;
    async fn next(&mut self) -> io::Result<Vec<Event>>;
    /// Complete accepted system cleanup within the existing shutdown safety deadline.
    async fn finish(&mut self, deadline: Instant) -> io::Result<()>;
}
fn control_permitted(
    uid: u32,
    command: &control::Command,
    session: Option<&crate::Session>,
) -> bool {
    uid == 0 || (*command != control::Command::Reload && session.is_some_and(|s| s.eligible(uid)))
}
#[derive(PartialEq)]
struct Policy {
    state: crate::machine::mode::State,
    session: Option<crate::Session>,
    owner: Option<u64>,
    recovery: bool,
}
pub struct Runtime<D> {
    daemon: Daemon,
    drivers: D,
    sockets: Sockets,
    clock: Instant,
    exiting: bool,
    logged: Option<Policy>,
    shutdown_deadline: Option<Instant>,
    repeats: logging::Repeats,
}
impl<D: Drivers> Runtime<D> {
    pub fn new(config: Config, recovery: bool, runtime: &Path, drivers: D) -> io::Result<Self> {
        let daemon = Daemon::new(config, recovery, Timestamp(0))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("{e:?}")))?;
        Ok(Self {
            daemon,
            drivers,
            sockets: Sockets::bind(runtime)?,
            clock: Instant::now(),
            exiting: false,
            logged: None,
            shutdown_deadline: None,
            repeats: Default::default(),
        })
    }
    fn log_policy(&mut self) {
        let d = &self.daemon;
        let changed = self.logged.as_ref().is_none_or(|p| {
            &p.state != d.mode().state()
                || p.session.as_ref() != d.session()
                || p.owner != d.owner()
                || p.recovery != d.recovery()
        });
        if changed {
            tracing::info!(state=?d.mode().state(),owner=?d.owner(),uid=?d.session().map(|s|s.uid),session=?d.session().map(|s|s.id.as_str()),recovery=d.recovery(),"daemon policy changed");
            self.logged = Some(Policy {
                state: d.mode().state().clone(),
                session: d.session().cloned(),
                owner: d.owner(),
                recovery: d.recovery(),
            });
        }
    }
    fn issue(&mut self, source: &str, message: impl std::fmt::Display) {
        for report in self
            .repeats
            .record(self.now(), source, &message.to_string())
        {
            report.emit();
        }
    }
    fn now(&self) -> Timestamp {
        Timestamp(u64::try_from(self.clock.elapsed().as_millis()).unwrap_or(u64::MAX))
    }
    pub fn status(&self) -> serde_json::Value {
        let m = self.daemon.mode();
        serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"mode":m.state(),"reason":self.daemon.mode_reason(),
            "device":{"id":m.device(),"state":m.state(),"driver":m.driver(),"failures":m.failures(),"error":m.error()},
            "owner":self.daemon.session(),"connection":self.daemon.owner(),"recovery":self.daemon.recovery(),
            "sleeping":self.daemon.sleeping(),"backlight":self.daemon.backlight(),"fn_bridge":self.daemon.bridge_active(),
            "inhibitor_error":self.daemon.inhibitor_error(),"drivers":self.drivers.status()})
    }
    fn event(&mut self, event: Event) {
        if matches!(event, Event::Shutdown) && self.shutdown_deadline.is_none() {
            self.shutdown_deadline = Some(Instant::now() + Duration::from_secs(3));
        }
        // Query new keys with the old bindings still installed. Merely holding a key across
        // reload cannot activate a newly installed recovery binding.
        let mut wanted = self.daemon.watch_keys();
        match &event {
            Event::Message {
                connection,
                message: SessionMessage::WatchKey { key, enabled: true },
            } if self.daemon.owner() == Some(*connection) => {
                wanted.insert(*key);
            }
            Event::Config(c) => {
                for b in &c.bindings {
                    wanted.extend(b.trigger.watch_keys());
                }
            }
            _ => {}
        }
        let mut commands = VecDeque::new();
        for e in self.drivers.watch_keys(wanted) {
            commands.extend(self.daemon.handle(self.now(), e));
        }
        commands.extend(self.daemon.handle(self.now(), event));
        self.commands(commands);
        let wanted = self.daemon.watch_keys();
        let mut commands = VecDeque::new();
        for e in self.drivers.watch_keys(wanted) {
            commands.extend(self.daemon.handle(self.now(), e));
        }
        self.commands(commands);
        self.log_policy();
    }
    fn commands(&mut self, mut commands: VecDeque<Command>) {
        while let Some(command) = commands.pop_front() {
            match command {
                Command::Flush { connection } => {
                    // Bounded by the model's 256-message queue plus terminal reserve.
                    while let Some(out) = self.daemon.pending(connection).cloned() {
                        let result = match out {
                            Outgoing::Message(message) => self.sockets.send(connection, message),
                            Outgoing::Swapchain { generation, header } => {
                                match self.drivers.buffers(generation) {
                                    Some(buffers) => {
                                        self.sockets.swapchain(connection, header, buffers)
                                    }
                                    None => Err(io::Error::new(
                                        io::ErrorKind::NotConnected,
                                        "swapchain generation unavailable",
                                    )),
                                }
                            }
                        };
                        match result {
                            Ok(()) => commands
                                .extend(self.daemon.handle(self.now(), Event::Sent { connection })),
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                            Err(_) => {
                                commands.extend(
                                    self.daemon
                                        .handle(self.now(), Event::Disconnected { connection }),
                                );
                                break;
                            }
                        }
                    }
                }
                Command::Close { connection } => self.sockets.close(connection),
                Command::Exit => {
                    self.drivers.dispatch(Command::Exit);
                    self.exiting = true;
                }
                other => {
                    for event in self.drivers.dispatch(other) {
                        commands.extend(self.daemon.handle(self.now(), event));
                    }
                }
            }
        }
    }
    fn incoming(&mut self, incoming: Incoming) {
        match incoming {
            Incoming::Session {
                connection,
                uid,
                message: SessionMessage::Hello { version },
            } => self.event(Event::Hello {
                connection,
                uid,
                version,
            }),
            Incoming::Session {
                connection,
                message,
                ..
            } => self.event(Event::Message {
                connection,
                message,
            }),
            Incoming::Disconnected { connection } => self.event(Event::Disconnected { connection }),
            Incoming::Writable { connection } => {
                self.commands(VecDeque::from([Command::Flush { connection }]))
            }
            Incoming::Diagnostic(error) => self.issue("socket accept", error),
            Incoming::Control {
                connection,
                uid,
                request,
            } => {
                let authorized = control_permitted(uid, &request.command, self.daemon.session());
                let reply = if !authorized {
                    control::Reply::Error("control request is not permitted for this UID".into())
                } else {
                    match request.command {
                        control::Command::Status => control::Reply::Status(self.status()),
                        control::Command::SetMode { mode } => {
                            self.event(Event::SetMode(mode));
                            control::Reply::Ack
                        }
                        control::Command::Reload => match self.drivers.reload() {
                            Ok(config) => {
                                self.event(Event::Config(config));
                                control::Reply::Ack
                            }
                            Err(e) => control::Reply::Error(e.to_string()),
                        },
                    }
                };
                if let Err(e) = self
                    .sockets
                    .respond(connection, control::Response::new(reply))
                {
                    self.issue("control response", e);
                }
            }
        }
    }
    pub async fn run(&mut self) -> io::Result<()> {
        let mut signals =
            async_signal::Signals::new([async_signal::Signal::Term, async_signal::Signal::Int])?;
        for event in self.drivers.watch_keys(self.daemon.watch_keys()) {
            self.event(event);
        }
        enum Ready {
            Driver(io::Result<Vec<Event>>),
            Socket(io::Result<Incoming>),
            Deadline,
            Signal,
        }
        let mut retry_driver = None;
        while !self.exiting {
            let deadline = self
                .daemon
                .next_deadline()
                .into_iter()
                .chain(self.repeats.deadline())
                .min()
                .map(|at| self.clock + Duration::from_millis(at.0));
            let ready = or(
                async {
                    if let Some(at) = retry_driver {
                        async_io::Timer::at(at).await;
                    }
                    Ready::Driver(self.drivers.next().await)
                },
                or(
                    async { Ready::Socket(self.sockets.next().await) },
                    or(
                        async {
                            match deadline {
                                Some(at) => {
                                    async_io::Timer::at(at).await;
                                }
                                None => pending::<()>().await,
                            };
                            Ready::Deadline
                        },
                        async {
                            let _ = signals.next().await;
                            Ready::Signal
                        },
                    ),
                ),
            )
            .await;
            match ready {
                Ready::Driver(Ok(events)) => {
                    retry_driver = None;
                    for e in events {
                        self.event(e);
                    }
                }
                Ready::Driver(Err(e)) => {
                    self.issue("driver event source", e);
                    retry_driver = Some(Instant::now() + Duration::from_millis(100));
                    self.event(Event::Shutdown);
                }
                Ready::Socket(Ok(incoming)) => self.incoming(incoming),
                Ready::Socket(Err(e)) => {
                    self.issue("socket source", e);
                    self.event(Event::Shutdown);
                }
                Ready::Deadline => {
                    for report in self.repeats.take_due(self.now()) {
                        report.emit();
                    }
                    let commands = self.daemon.advance(self.now());
                    self.commands(commands.into());
                    self.log_policy();
                }
                Ready::Signal => self.event(Event::Shutdown),
            }
        }
        self.drivers
            .finish(
                self.shutdown_deadline
                    .unwrap_or_else(|| Instant::now() + Duration::from_secs(3)),
            )
            .await
    }
}

pub mod linux;
pub mod marker;

pub mod logging;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_permissions_follow_active_session_and_reload_is_root_only() {
        let session = crate::Session {
            uid: 1000,
            id: "s".into(),
            user_display: Some("s".into()),
            active: true,
            state: "active".into(),
            class: "user".into(),
            kind: "wayland".into(),
            locked: false,
        };
        assert!(control_permitted(0, &control::Command::Reload, None));
        assert!(control_permitted(
            1000,
            &control::Command::Status,
            Some(&session)
        ));
        assert!(control_permitted(
            1000,
            &control::Command::SetMode {
                mode: touchbar_protocol::Mode::Native
            },
            Some(&session)
        ));
        assert!(!control_permitted(
            1000,
            &control::Command::Reload,
            Some(&session)
        ));
        assert!(!control_permitted(
            1001,
            &control::Command::Status,
            Some(&session)
        ));
        let mut greeter = session.clone();
        greeter.class = "greeter".into();
        assert!(!control_permitted(
            1000,
            &control::Command::Status,
            Some(&greeter)
        ));
        assert!(!control_permitted(1000, &control::Command::Status, None));
    }
}
