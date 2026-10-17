use super::{
    connection::{Client, Contact, MAX_CONNECTIONS, MAX_CONTACTS, QUEUE_LIMIT},
    mode::{self, ModeMachine},
    power::{Bridge, Power},
    types::*,
};
use crate::config::Config;
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
};
use touchbar_bindings::{
    BindingEngine, BindingEvent, BindingIssue, KeyInput, config::RecoveryAction,
};
use touchbar_model::{FactName, FactValue, KeyPhase, Timestamp, TouchPhase, WatchKey};
use touchbar_protocol::{
    DaemonMessage as Message, MAX_FRAME_DAMAGE, Mode, ModeReason, RevokedReason, SessionMessage,
    VERSION,
};
#[derive(Clone, Debug)]
struct Grace {
    uid: u32,
    session: String,
    deadline: Timestamp,
}
#[derive(Clone, Debug)]
pub struct Daemon {
    config: Config,
    mode: ModeMachine,
    session: Option<Session>,
    owner: Option<u64>,
    clients: BTreeMap<u64, Client>,
    last_connection: u64,
    grace: Option<Grace>,
    recovery: bool,
    sleeping: bool,
    sleep_at: Option<Timestamp>,
    inhibitor: Option<u64>,
    inhibitor_pending: Option<u64>,
    inhibitor_serial: u64,
    inhibitor_error: Option<String>,
    started: bool,
    stopping: bool,
    stop_at: Option<Timestamp>,
    stopped: bool,
    bindings: BindingEngine<RecoveryAction>,
    physical: BTreeSet<WatchKey>,
    power: Power,
    bridge: Bridge,
    autodim_owned: bool,
    last_level: Option<u8>,
    driver_timeouts: (Option<u64>, Option<u64>),
    wake_contacts: BTreeSet<u32>,
    now: Timestamp,
    device_commands: Vec<mode::Command>,
    flush: BTreeSet<u64>,
}
impl Daemon {
    pub fn new(config: Config, recovery: bool, now: Timestamp) -> Result<Self, Vec<BindingIssue>> {
        let bindings = BindingEngine::new(config.bindings.clone())?;
        let power = Power::new(config.backlight, now);
        let mut d = Self {
            config,
            mode: ModeMachine::new(),
            session: None,
            owner: None,
            clients: BTreeMap::new(),
            last_connection: 0,
            grace: None,
            recovery,
            sleeping: false,
            sleep_at: None,
            inhibitor: None,
            inhibitor_pending: None,
            inhibitor_serial: 0,
            inhibitor_error: None,
            started: false,
            stopping: false,
            stop_at: None,
            stopped: false,
            bindings,
            physical: BTreeSet::new(),
            power,
            bridge: Bridge::default(),
            autodim_owned: false,
            last_level: None,
            driver_timeouts: (None, None),
            wake_contacts: BTreeSet::new(),
            now,
            device_commands: Vec::new(),
            flush: BTreeSet::new(),
        };
        d.set_timeouts();
        let _ = d.bindings.set_facts(now, d.facts());
        Ok(d)
    }
    pub fn inhibitor_error(&self) -> Option<&str> {
        self.inhibitor_error.as_deref()
    }
    pub fn owner(&self) -> Option<u64> {
        self.owner
    }
    pub fn mode(&self) -> &ModeMachine {
        &self.mode
    }
    pub fn recovery(&self) -> bool {
        self.recovery
    }
    pub fn sleeping(&self) -> bool {
        self.sleeping
    }
    pub fn backlight(&self) -> u8 {
        self.power.level(self.now)
    }
    pub fn pending(&self, connection: u64) -> Option<&Outgoing> {
        self.clients.get(&connection)?.queue.front()
    }
    pub fn queued(&self, connection: u64) -> impl Iterator<Item = &Outgoing> {
        self.clients
            .get(&connection)
            .into_iter()
            .flat_map(|c| c.queue.iter())
    }
    pub fn queue_len(&self, connection: u64) -> usize {
        self.clients.get(&connection).map_or(0, |c| c.queue.len())
    }
    fn eligible(&self, uid: u32, id: &str) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.eligible(uid) && s.id == id)
    }
    fn permitted(&self) -> bool {
        self.session.as_ref().is_some_and(|s| !s.locked)
            && !self.recovery
            && !self.sleeping
            && !self.stopping
    }
    pub fn target(&self) -> Mode {
        let owner = self
            .owner
            .and_then(|id| self.clients.get(&id))
            .is_some_and(|c| {
                c.session
                    .as_ref()
                    .is_some_and(|id| self.eligible(c.uid, id))
            });
        let grace = self
            .grace
            .as_ref()
            .is_some_and(|g| g.deadline > self.now && self.eligible(g.uid, &g.session))
            && matches!(self.mode.state(), mode::State::Display { .. });
        if self.permitted() && (owner || grace) {
            self.mode.target_for_available_device(Mode::Display)
        } else {
            Mode::Native
        }
    }
    fn mode_input(&mut self, input: mode::Input) {
        self.device_commands
            .extend(self.mode.handle(self.now, input));
    }
    pub fn handle(&mut self, now: Timestamp, event: Event) -> Vec<Command> {
        if self.stopped {
            return Vec::new();
        }
        self.now = self.now.max(now);
        let mut out = Vec::new();
        match event {
            Event::Start { preparing_sleep } if !self.started => {
                self.started = true;
                self.prepare_sleep(preparing_sleep, &mut out);
                if !preparing_sleep {
                    self.acquire(&mut out);
                }
            }
            Event::Session(session) => {
                self.session = session;
                if let Some(id) = self.owner
                    && !self.client_eligible(id)
                {
                    self.close(id, Some(RevokedReason::SessionInactive), false, &mut out);
                }
            }
            Event::Hello {
                connection,
                uid,
                version,
            } => self.hello(connection, uid, version, &mut out),
            Event::Disconnected { connection } => {
                self.close(connection, None, true, &mut out);
                self.clients.remove(&connection);
                self.flush.remove(&connection);
                out.push(Command::Close { connection });
            }
            Event::ProtocolError { connection } => self.close(connection, None, true, &mut out),
            Event::Sent { connection } => {
                if let Some(c) = self.clients.get_mut(&connection) {
                    let sent = c.queue.pop_front();
                    if let Some(index) = c.enable_after {
                        if index == 0 {
                            c.input_enabled = c.display
                                && matches!(
                                    sent,
                                    Some(Outgoing::Message(Message::ModeChanged {
                                        mode: Mode::Display,
                                        ..
                                    }))
                                );
                            c.enable_after = None;
                        } else {
                            c.enable_after = Some(index - 1);
                        }
                    }
                    if c.queue.is_empty() && c.closing {
                        self.clients.remove(&connection);
                        out.push(Command::Close { connection });
                    }
                }
            }
            Event::Message {
                connection,
                message,
            } if self.owner == Some(connection) => self.message(connection, message, &mut out),
            Event::Device(device) => {
                if device != self.mode.device() {
                    self.grace = None;
                    self.wake_contacts.clear();
                }
                self.mode_input(mode::Input::Device(device));
            }
            Event::DisplayReady { generation, info } => {
                if matches!(self.mode.state(),mode::State::ToDisplay{generation:g,..} if *g==generation)
                {
                    if info.valid() {
                        self.mode_input(mode::Input::DisplayReady { generation });
                        if matches!(self.mode.state(),mode::State::Display{generation:g} if *g==generation)
                        {
                            self.announce_display(generation, info, &mut out);
                        }
                    } else {
                        self.mode_input(mode::Input::Failed {
                            generation,
                            message: "invalid display metadata".into(),
                        });
                    }
                }
            }
            Event::NativeSafe { generation } => {
                self.mode_input(mode::Input::NativeSafe { generation })
            }
            Event::TransitionFailed {
                generation,
                message,
            } => self.mode_input(mode::Input::Failed {
                generation,
                message,
            }),
            Event::Driver { device, mode } => self.mode_input(mode::Input::Driver { device, mode }),
            Event::SetMode(mode) => self.set_recovery(mode == Mode::Native, &mut out),
            Event::PrepareForSleep(preparing) => self.prepare_sleep(preparing, &mut out),
            Event::InhibitorFailed {
                generation,
                message,
            } => {
                if self.inhibitor_pending == Some(generation) || self.inhibitor == Some(generation)
                {
                    self.inhibitor_pending = None;
                    self.inhibitor = None;
                    self.inhibitor_error = Some(message);
                }
            }
            Event::InhibitorAcquired { generation } => {
                if self.inhibitor_pending == Some(generation) {
                    self.inhibitor_pending = None;
                    self.inhibitor = Some(generation);
                    self.inhibitor_error = None;
                } else {
                    out.push(Command::ReleaseInhibitor { generation });
                }
            }
            Event::Shutdown if !self.stopping => {
                self.stopping = true;
                self.stop_at = Some(self.now.after(3000));
                self.grace = None;
                self.bindings.reset();
                if let Some(id) = self.owner {
                    self.close(id, Some(RevokedReason::Shutdown), false, &mut out);
                }
                if self.autodim_owned {
                    self.autodim_owned = false;
                    out.push(Command::RestoreAutodim);
                }
            }
            Event::PhysicalKey(input) => self.key(input, &mut out),
            Event::VirtualActivity | Event::Activity => self.power.activity(self.now),
            Event::Lid(closed) => self.power.lid(closed),
            Event::DriverTimeouts {
                dim_seconds,
                idle_seconds,
            } => {
                self.driver_timeouts = (dim_seconds, idle_seconds);
                self.set_timeouts();
            }
            Event::Autodim(true) if !self.autodim_owned && !self.stopping => {
                self.autodim_owned = true;
                out.push(Command::OwnAutodim);
            }
            Event::Touch {
                tracking,
                phase,
                x,
                y,
            } => self.touch(tracking, phase, x, y, &mut out),
            Event::FramePresented {
                generation,
                connection,
                index,
                frame,
                commit_us,
            } => {
                if generation == self.mode.generation()
                    && self.owner == Some(connection)
                    && let Some(c) = self.clients.get_mut(&connection)
                    && c.display
                    && c.pending_frames.front() == Some(&(index, frame))
                    && i32::try_from(commit_us).is_ok()
                {
                    c.pending_frames.pop_front();
                    self.enqueue(
                        connection,
                        Outgoing::Message(Message::FramePresented {
                            index,
                            frame,
                            commit_us,
                        }),
                        &mut out,
                    );
                }
            }
            Event::FrameReleased {
                generation,
                connection,
                index,
                frame,
            } => {
                if generation == self.mode.generation()
                    && self.owner == Some(connection)
                    && let Some(c) = self.clients.get_mut(&connection)
                    && c.display
                    && c.frames.get(&index) == Some(&frame)
                    && !c.pending_frames.contains(&(index, frame))
                {
                    c.frames.remove(&index);
                    self.enqueue(
                        connection,
                        Outgoing::Message(Message::BufferReleased { index, frame }),
                        &mut out,
                    );
                }
            }
            Event::Config(config) => {
                if let Ok(bindings) = BindingEngine::new(config.bindings.clone()) {
                    self.bindings = bindings;
                    self.config = config;
                    self.power.levels(self.config.backlight);
                    self.set_timeouts();
                }
            }
            _ => {}
        }
        self.finish(out)
    }
    pub fn advance(&mut self, now: Timestamp) -> Vec<Command> {
        if self.stopped {
            return Vec::new();
        }
        self.now = self.now.max(now);
        self.finish(Vec::new())
    }
    fn facts(&self) -> touchbar_model::Facts {
        [
            (
                FactName::from("mode"),
                FactValue::Text(
                    if matches!(self.mode.state(), mode::State::Display { .. }) {
                        "display"
                    } else {
                        "native"
                    }
                    .into(),
                ),
            ),
            (
                FactName::from("recovery"),
                FactValue::Boolean(self.recovery),
            ),
        ]
        .into()
    }
    fn finish(&mut self, mut out: Vec<Command>) -> Vec<Command> {
        let expired: Vec<_> = self
            .clients
            .iter()
            .filter_map(|(id, c)| c.close_at.filter(|at| *at <= self.now).map(|_| *id))
            .collect();
        for connection in expired {
            self.clients.remove(&connection);
            self.flush.remove(&connection);
            out.push(Command::Close { connection });
        }
        if !self.permitted()
            || self
                .grace
                .as_ref()
                .is_some_and(|g| g.deadline <= self.now || !self.eligible(g.uid, &g.session))
        {
            self.grace = None;
        }
        let events = self.bindings.set_facts(self.now, self.facts());
        self.binding_events(events, &mut out);
        self.mode_input(mode::Input::Target(self.target()));
        let events = self.bindings.set_facts(self.now, self.facts());
        self.binding_events(events, &mut out);
        self.mode_input(mode::Input::Target(self.target()));
        // Target update also advances transition deadlines; no periodic timer is needed.
        if !matches!(self.mode.state(), mode::State::Display { .. })
            && let Some(id) = self.owner
            && self.clients.get(&id).is_some_and(|c| c.display)
        {
            self.quiesce(id, &mut out);
            let reason = self.reason();
            self.enqueue(
                id,
                Outgoing::Message(Message::ModeChanged {
                    mode: Mode::Native,
                    reason,
                }),
                &mut out,
            );
        }
        let native =
            matches!(self.mode.state(), mode::State::Native { safe: true }) && !self.stopping;
        let driver = if let mode::DriverState::Ready { mode } = self.mode.driver() {
            Some(*mode)
        } else {
            None
        };
        if let Some(mode) =
            self.bridge
                .reconcile(native, self.physical.contains(&WatchKey::Fn), driver)
        {
            out.push(Command::WriteDriverMode(mode));
        }
        let level = self.power.level(self.now);
        if !self.stopping && self.last_level != Some(level) {
            self.last_level = Some(level);
            out.push(Command::SetBacklight(level));
        }
        if self.sleeping
            && (self.mode.native_safe()
                || self.sleep_at.is_some_and(|at| at.after(4000) <= self.now))
        {
            self.release_inhibitor(&mut out);
        }
        let native_done =
            matches!(self.mode.state(), mode::State::Native { .. }) && self.mode.discovered();
        let exit = self.stopping && (native_done || self.stop_at.is_some_and(|at| at <= self.now));
        if exit {
            self.release_inhibitor(&mut out);
        }
        out.extend(self.device_commands.drain(..).map(Command::Device));
        out.extend(
            std::mem::take(&mut self.flush)
                .into_iter()
                .map(|connection| Command::Flush { connection }),
        );
        if exit {
            self.stopped = true;
            out.push(Command::Exit);
        }
        out
    }
    fn reason(&self) -> ModeReason {
        if self.sleeping {
            ModeReason::Sleep
        } else if self.recovery {
            ModeReason::Recovery
        } else if self.session.as_ref().is_some_and(|s| s.locked) {
            ModeReason::Locked
        } else if self.mode.error().is_some() {
            ModeReason::Error
        } else {
            ModeReason::Owner
        }
    }
    fn client_eligible(&self, id: u64) -> bool {
        self.clients
            .get(&id)
            .is_some_and(|c| c.session.as_ref().is_some_and(|s| self.eligible(c.uid, s)))
    }
    fn hello(&mut self, id: u64, uid: u32, version: u16, out: &mut Vec<Command>) {
        if self.clients.contains_key(&id) {
            self.close(id, None, true, out);
            return;
        }
        if id <= self.last_connection || self.clients.len() >= MAX_CONNECTIONS {
            out.push(Command::Close { connection: id });
            return;
        }
        self.last_connection = id;
        self.clients.insert(id, Client::new(uid));
        let reject = if self.stopping {
            Some(RevokedReason::Shutdown)
        } else if version != VERSION {
            Some(RevokedReason::Version)
        } else if !self.session.as_ref().is_some_and(|s| s.eligible(uid)) {
            Some(RevokedReason::NotActive)
        } else {
            None
        };
        if let Some(reason) = reject {
            self.close(id, Some(reason), false, out);
            return;
        }
        if let Some(old) = self.owner {
            self.close(old, Some(RevokedReason::Replaced), false, out);
        }
        self.grace = None;
        self.owner = Some(id);
        self.power.limit(u16::MAX);
        self.clients.get_mut(&id).expect("new client").session =
            Some(self.session.as_ref().expect("eligible").id.clone());
        self.enqueue(
            id,
            Outgoing::Message(Message::ModeChanged {
                mode: Mode::Native,
                reason: self.reason(),
            }),
            out,
        );
        // A new connection receives a fresh swapchain, without a USB mode round trip during grace.
        self.mode_input(mode::Input::RefreshDisplay);
    }
    fn announce_display(&mut self, generation: u64, info: DisplayInfo, out: &mut Vec<Command>) {
        let Some(id) = self.owner else {
            return;
        };
        let c = self.clients.get_mut(&id).expect("owner exists");
        if c.display {
            return;
        }
        c.display = true;
        c.info = Some(info);
        self.enqueue(
            id,
            Outgoing::Message(Message::Welcome {
                caps: info.caps,
                width: info.header.logical_width,
                height: info.header.logical_height,
                width_mm100: info.width_mm100,
                height_mm100: info.height_mm100,
            }),
            out,
        );
        let enable_after = self.clients[&id].queue.len();
        self.enqueue(
            id,
            Outgoing::Message(Message::ModeChanged {
                mode: Mode::Display,
                reason: self.reason(),
            }),
            out,
        );
        if let Some(c) = self.clients.get_mut(&id).filter(|c| !c.closing) {
            c.enable_after = Some(enable_after);
            c.input_enabled = false;
        }
        self.enqueue(
            id,
            Outgoing::Swapchain {
                generation,
                header: info.header,
            },
            out,
        );
    }
    fn set_recovery(&mut self, value: bool, out: &mut Vec<Command>) {
        if value != self.recovery {
            self.recovery = value;
            if value {
                self.grace = None;
            }
            out.push(Command::WriteRecovery(value));
        }
    }
    fn binding_events(
        &mut self,
        events: Vec<BindingEvent<RecoveryAction>>,
        out: &mut Vec<Command>,
    ) {
        for event in events {
            let action = match event {
                BindingEvent::Fired { action, .. } | BindingEvent::Began { action, .. } => action,
                BindingEvent::Ended { .. } => continue,
            };
            self.set_recovery(
                match action {
                    RecoveryAction::Native => true,
                    RecoveryAction::Display => false,
                    RecoveryAction::ToggleRecovery => !self.recovery,
                },
                out,
            );
        }
    }
    fn key(&mut self, input: KeyInput, out: &mut Vec<Command>) {
        if self.stopping {
            return;
        }
        self.power.activity(self.now);
        let changed = match input.phase {
            KeyPhase::Pressed => self.physical.insert(input.code),
            KeyPhase::Released => self.physical.remove(&input.code),
        };
        if changed {
            let events = self.bindings.handle(self.now, input);
            self.binding_events(events, out);
            if let Some(id) = self.owner
                && self
                    .clients
                    .get(&id)
                    .is_some_and(|c| c.watched.contains(&input.code))
            {
                self.enqueue(
                    id,
                    Outgoing::Message(Message::KeyEvent {
                        key: input.code,
                        phase: input.phase,
                    }),
                    out,
                );
            }
        }
    }
    fn set_timeouts(&mut self) {
        self.power.set_timeouts(
            self.config
                .dim_timeout
                .or(self.driver_timeouts.0)
                .and_then(|s| s.checked_mul(1000)),
            self.config
                .idle_timeout
                .or(self.driver_timeouts.1)
                .and_then(|s| s.checked_mul(1000)),
        );
    }
    fn prepare_sleep(&mut self, preparing: bool, out: &mut Vec<Command>) {
        if preparing && !self.sleeping {
            self.sleep_at = Some(self.now);
            self.grace = None;
        }
        self.sleeping = preparing;
        if !preparing {
            self.sleep_at = None;
            if self.started {
                self.acquire(out);
            }
        }
    }
    fn acquire(&mut self, out: &mut Vec<Command>) {
        if self.inhibitor.is_none() && self.inhibitor_pending.is_none() && !self.stopping {
            self.inhibitor_serial += 1;
            self.inhibitor_pending = Some(self.inhibitor_serial);
            out.push(Command::AcquireInhibitor {
                generation: self.inhibitor_serial,
            });
        }
    }
    fn release_inhibitor(&mut self, out: &mut Vec<Command>) {
        if let Some(generation) = self.inhibitor.take() {
            out.push(Command::ReleaseInhibitor { generation });
        }
    }
    fn quiesce(&mut self, id: u64, out: &mut Vec<Command>) {
        let Some(c) = self.clients.get_mut(&id) else {
            return;
        };
        for key in std::mem::take(&mut c.pressed) {
            out.push(Command::Uinput {
                key,
                phase: KeyPhase::Released,
            });
        }
        for contact in std::mem::take(&mut c.contacts).into_values() {
            c.queue.push_back(Outgoing::Message(Message::Touch {
                phase: TouchPhase::Cancel,
                contact: contact.id,
                x: contact.x,
                y: contact.y,
            }));
        }
        c.display = false;
        c.input_enabled = false;
        c.enable_after = None;
        c.damage.clear();
        c.frames.clear();
        c.pending_frames.clear();
        c.last_frame = None;
        c.info = None;
        if !c.queue.is_empty() {
            self.flush.insert(id);
        }
    }
    fn close(
        &mut self,
        id: u64,
        reason: Option<RevokedReason>,
        grace: bool,
        out: &mut Vec<Command>,
    ) {
        if self.clients.get(&id).is_none_or(|c| c.closing) {
            return;
        }
        let was_owner = self.owner == Some(id);
        if was_owner
            && grace
            && self.config.session_grace_ms > 0
            && self.permitted()
            && matches!(self.mode.state(), mode::State::Display { .. })
            && self.client_eligible(id)
        {
            let c = &self.clients[&id];
            self.grace = Some(Grace {
                uid: c.uid,
                session: c.session.clone().expect("owner session"),
                deadline: self.now.after(self.config.session_grace_ms),
            });
        }
        self.quiesce(id, out);
        let c = self.clients.get_mut(&id).expect("client exists");
        c.closing = true;
        c.close_at = Some(self.now.after(1000));
        if let Some(reason) = reason {
            c.queue
                .push_back(Outgoing::Message(Message::Revoked(reason)));
            self.flush.insert(id);
        }
        if was_owner {
            self.owner = None;
            self.power.limit(u16::MAX);
        }
        if self.clients[&id].queue.is_empty() {
            self.clients.remove(&id);
            out.push(Command::Close { connection: id });
        }
    }
    fn enqueue(&mut self, id: u64, message: Outgoing, out: &mut Vec<Command>) {
        let Some(c) = self.clients.get_mut(&id).filter(|c| !c.closing) else {
            return;
        };
        let motion = if let Outgoing::Message(Message::Touch {
            phase: TouchPhase::Move,
            contact,
            ..
        }) = message
        {
            Some(contact)
        } else {
            None
        };
        if let Some(contact) = motion {
            if let Some(pos)=c.queue.iter().rposition(|v|matches!(v,Outgoing::Message(Message::Touch{phase:TouchPhase::Move,contact:id,..}) if *id==contact)) {c.queue.remove(pos);}
            else if c.queue.len()>=QUEUE_LIMIT {return;}
        }
        c.queue.push_back(message);
        self.flush.insert(id);
        if c.queue.len() > QUEUE_LIMIT {
            self.close(id, None, true, out);
        }
    }
    fn message(&mut self, id: u64, message: SessionMessage, out: &mut Vec<Command>) {
        match message {
            SessionMessage::Hello { .. } => self.close(id, None, true, out),
            SessionMessage::WatchKey { key, enabled } => {
                let c = self.clients.get_mut(&id).expect("owner");
                if enabled {
                    c.watched.insert(key);
                } else {
                    c.watched.remove(&key);
                }
                let phase = if enabled && self.physical.contains(&key) {
                    KeyPhase::Pressed
                } else {
                    KeyPhase::Released
                };
                self.enqueue(id, Outgoing::Message(Message::KeyEvent { key, phase }), out);
            }
            SessionMessage::Backlight { limit } => self.power.limit(limit),
            SessionMessage::Key { key, phase } => {
                let c = self.clients.get_mut(&id).expect("owner");
                let changed = match phase {
                    KeyPhase::Pressed
                        if c.display
                            && c.input_enabled
                            && matches!(self.mode.state(), mode::State::Display { .. }) =>
                    {
                        c.pressed.insert(key)
                    }
                    KeyPhase::Released => c.pressed.remove(&key),
                    _ => false,
                };
                if changed {
                    out.push(Command::Uinput { key, phase });
                }
            }
            SessionMessage::FrameDamage(rect) => {
                let c = self.clients.get_mut(&id).expect("owner");
                if !c.display
                    || c.damage.len() >= MAX_FRAME_DAMAGE
                    || c.info.is_none_or(|i| {
                        !i.header.frame_damage
                            || !rect.fits(i.header.physical_width, i.header.physical_height)
                    })
                {
                    self.close(id, None, true, out);
                } else {
                    c.damage.push(rect);
                }
            }
            SessionMessage::FrameReady { index, frame } => {
                let c = self.clients.get_mut(&id).expect("owner");
                if !c.display
                    || !c.input_enabled
                    || !matches!(self.mode.state(), mode::State::Display { .. })
                    || c.info.is_none_or(|i| {
                        index >= u32::from(i.header.buffer_count) || !c.pending_frames.is_empty()
                    })
                    || c.frames.contains_key(&index)
                    || c.last_frame.is_some_and(|last| frame <= last)
                {
                    self.close(id, None, true, out);
                } else {
                    c.frames.insert(index, frame);
                    c.pending_frames.push_back((index, frame));
                    c.last_frame = Some(frame);
                    out.push(Command::Present {
                        generation: self.mode.generation(),
                        connection: id,
                        index,
                        frame,
                        damage: std::mem::take(&mut c.damage),
                    });
                }
            }
        }
    }
    fn touch(&mut self, tracking: u32, phase: TouchPhase, x: i32, y: i32, out: &mut Vec<Command>) {
        if phase == TouchPhase::Down
            && let Some(id) = self.owner
            && self.clients[&id].contacts.contains_key(&tracking)
        {
            self.close(id, None, true, out);
            return;
        }
        let off = self.power.level(self.now) == 0;
        self.power.activity(self.now);
        if phase == TouchPhase::Down && off {
            if self.wake_contacts.len() < MAX_CONTACTS {
                self.wake_contacts.insert(tracking);
            } else {
                return;
            }
        }
        if self.wake_contacts.contains(&tracking) {
            if matches!(phase, TouchPhase::Up | TouchPhase::Cancel) {
                self.wake_contacts.remove(&tracking);
            }
            return;
        }
        let Some(id) = self.owner else {
            return;
        };
        let c = self.clients.get_mut(&id).expect("owner");
        if !c.display
            || !c.input_enabled
            || !matches!(self.mode.state(), mode::State::Display { .. })
        {
            return;
        }
        let contact = if phase == TouchPhase::Down {
            if c.contacts.contains_key(&tracking) || c.contacts.len() >= MAX_CONTACTS {
                self.close(id, None, true, out);
                return;
            }
            let Some(contact) = u32::try_from(c.next_contact).ok().and_then(NonZeroU32::new) else {
                self.close(id, None, true, out);
                return;
            };
            c.next_contact += 1;
            let entry = Contact { id: contact, x, y };
            c.contacts.insert(tracking, entry.clone());
            entry
        } else {
            let Some(contact) = c.contacts.get_mut(&tracking) else {
                return;
            };
            contact.x = x;
            contact.y = y;
            let contact = contact.clone();
            if matches!(phase, TouchPhase::Up | TouchPhase::Cancel) {
                c.contacts.remove(&tracking);
            }
            contact
        };
        self.enqueue(
            id,
            Outgoing::Message(Message::Touch {
                phase,
                contact: contact.id,
                x,
                y,
            }),
            out,
        );
    }
    /// Cheap policy checks for tests/diagnostics, never a full daemon snapshot.
    pub fn check_invariants(&self) -> Result<(), &'static str> {
        if matches!(self.mode.state(), mode::State::Display { .. })
            && self.target() != Mode::Display
        {
            return Err("T1: display without eligibility");
        }
        if self.clients.len() > MAX_CONNECTIONS
            || self
                .owner
                .is_some_and(|id| !self.client_eligible(id) || self.clients[&id].closing)
        {
            return Err("T2: invalid owner or unbounded clients");
        }
        for client in self.clients.values() {
            if client.queue.len() > QUEUE_LIMIT + MAX_CONTACTS + 2 {
                return Err("T6: unbounded terminal queue");
            }
            let mut revoked = false;
            for message in &client.queue {
                if revoked {
                    return Err("T2: message after Revoked");
                }
                revoked = matches!(message, Outgoing::Message(Message::Revoked(_)));
            }
            if (!client.display || client.closing)
                && (!client.pressed.is_empty() || !client.contacts.is_empty())
            {
                return Err("T3/T4/T5: inputs survived quiesce");
            }
            if client.contacts.values().any(|c| c.id.get() == 0)
                || client.contacts.len() > MAX_CONTACTS
            {
                return Err("T4: invalid contacts");
            }
            if client
                .contacts
                .keys()
                .any(|key| self.wake_contacts.contains(key))
            {
                return Err("T9: wake contact delivered");
            }
            if client.info.is_some_and(|i| {
                client.frames.len() > usize::from(i.header.buffer_count)
                    || client.pending_frames.len() > 1
            }) {
                return Err("too many in-flight buffers");
            }
        }
        if self.wake_contacts.len() > MAX_CONTACTS {
            return Err("T9: unbounded wake contacts");
        }
        if self.bridge.active()
            && (!matches!(self.mode.state(), mode::State::Native { safe: true })
                || self.stopping
                || !self.physical.contains(&WatchKey::Fn))
        {
            return Err("T7: Fn override survived native/hold");
        }
        if self.stopped && self.autodim_owned {
            return Err("T8: autodim not restored");
        }
        if self.next_deadline().is_some_and(|at| at <= self.now) {
            return Err("T10: overdue/spinning deadline");
        }
        Ok(())
    }
    pub fn next_deadline(&self) -> Option<Timestamp> {
        if self.stopped {
            return None;
        }
        let sleep = if self.sleeping && self.inhibitor.is_some() && !self.mode.native_safe() {
            self.sleep_at
                .map(|at| at.after(4000))
                .filter(|at| *at > self.now)
        } else {
            None
        };
        [
            self.mode.next_deadline(),
            self.grace.as_ref().map(|g| g.deadline),
            self.bindings.next_deadline(),
            self.power.next_deadline(self.now),
            sleep,
            self.stop_at,
            self.clients.values().filter_map(|c| c.close_at).min(),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}
