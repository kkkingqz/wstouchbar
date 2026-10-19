use crate::{Clock, control::Timing, surface::SurfaceEvent};
use async_io::Async;
use std::{
    cell::RefCell,
    collections::{BTreeSet, VecDeque},
    io,
    os::fd::OwnedFd,
    rc::Rc,
};
use touchbar_core::{Input, KeySource};
use touchbar_model::*;
use touchbar_protocol::{
    self as wire, DaemonMessage as Out, DaemonPacket, Mode, ModeReason, SessionMessage as In,
    SwapchainHeader,
};
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
#[derive(Clone, Debug)]
pub enum ClientEvent {
    Welcome(SurfaceInfo),
    Surface(SurfaceEvent),
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct Status {
    pub connected: bool,
    pub mode: Option<Mode>,
    pub reason: Option<ModeReason>,
    pub path: Option<String>,
    pub commit_us: Timing,
    pub ack_ms: Timing,
    pub error: Option<String>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            connected: false,
            mode: None,
            reason: None,
            path: None,
            commit_us: Timing::from(&VecDeque::new()),
            ack_ms: Timing::from(&VecDeque::new()),
            error: None,
        }
    }
}
pub struct TargetSlot {
    pub header: SwapchainHeader,
    pub buffers: Option<Vec<OwnedFd>>,
    pub index: u32,
    pub force_full: bool,
}
struct Lease {
    frame: u64,
    presented: bool,
}
struct Pending {
    index: u32,
    frame: u64,
    generation: Generation,
    sent: Option<Timestamp>,
}
struct State {
    id: SurfaceId,
    info: Option<SurfaceInfo>,
    mode: Mode,
    status: Status,
    events: VecDeque<ClientEvent>,
    outgoing: VecDeque<In>,
    watched: BTreeSet<WatchKey>,
    header: Option<SwapchainHeader>,
    buffers: Option<Vec<OwnedFd>>,
    held: Vec<Option<Lease>>,
    prepared: Option<u32>,
    pending: Option<Pending>,
    next_frame: u64,
    full: bool,
    deferred: bool,
    closed: bool,
    commit: VecDeque<f64>,
    ack: VecDeque<f64>,
    wake: async_channel::Sender<()>,
}
impl State {
    fn enqueue(&mut self, m: In) -> io::Result<()> {
        if self.closed {
            return Err(io::ErrorKind::NotConnected.into());
        }
        if self.outgoing.len() >= 256 {
            self.closed = true;
            let _ = self.wake.try_send(());
            return Err(invalid("touchbard outgoing queue is full"));
        }
        self.outgoing.push_back(m);
        let _ = self.wake.try_send(());
        Ok(())
    }
    fn input(&mut self, input: Input) {
        self.events
            .push_back(ClientEvent::Surface(SurfaceEvent::Input(input)))
    }
    fn retry(&mut self) {
        if self.deferred && self.mode == Mode::Display {
            self.deferred = false;
            self.events
                .push_back(ClientEvent::Surface(SurfaceEvent::RetryPresent(self.id)));
        }
    }
    fn native(&mut self) {
        self.header = None;
        self.buffers = None;
        self.held.clear();
        self.pending = None;
        self.prepared = None;
        self.full = true;
        self.deferred = false;
        self.status.path = None;
        self.outgoing.retain(|m| {
            !matches!(
                m,
                In::FrameReady { .. }
                    | In::FrameDamage(_)
                    | In::Key {
                        phase: KeyPhase::Pressed,
                        ..
                    }
            )
        });
    }
    fn sample(samples: &mut VecDeque<f64>, value: f64) {
        if samples.len() == 512 {
            samples.pop_front();
        }
        samples.push_back(value);
    }
    fn packet(&mut self, packet: DaemonPacket, now: Timestamp) -> io::Result<()> {
        match packet {
            DaemonPacket::Swapchain { header, buffers } => {
                let info = self
                    .info
                    .as_ref()
                    .ok_or_else(|| invalid("swapchain before Welcome"))?;
                if self.header.is_some()
                    || (header.logical_width, header.logical_height)
                        != (info.width_px, info.height_px)
                {
                    return Err(invalid("unexpected swapchain geometry or replacement"));
                }
                self.held = (0..header.buffer_count).map(|_| None).collect();
                self.header = Some(header);
                self.buffers = Some(buffers);
                self.next_frame = 1;
                self.full = true;
                self.retry();
            }
            DaemonPacket::Message(message) => match message {
                Out::Welcome {
                    caps,
                    width,
                    height,
                    width_mm100,
                    ..
                } => {
                    if self.header.is_some() {
                        return Err(invalid("Welcome while swapchain is active"));
                    }
                    let info = SurfaceInfo {
                        id: self.id,
                        name: "touchbar".parse().expect("fixed valid name"),
                        kind: SurfaceKind::Touchbar,
                        width_px: width,
                        height_px: height,
                        px_per_mm: width as f32 * 100.0 / width_mm100 as f32,
                        caps: SurfaceCaps {
                            modes: true,
                            backlight: true,
                            watch_keys: true,
                            inject_keys: true,
                            damage: caps & 1 != 0,
                        },
                    };
                    info.validate().map_err(|e| invalid(&e.to_string()))?;
                    if self.info.is_some() {
                        self.input(Input::SurfaceChanged(info.clone()));
                    } else {
                        self.events.push_back(ClientEvent::Welcome(info.clone()));
                    }
                    self.info = Some(info);
                    self.input(Input::Mode {
                        surface: self.id,
                        mode: if self.mode == Mode::Native {
                            touchbar_core::Mode::Native
                        } else {
                            touchbar_core::Mode::Display
                        },
                    });
                }
                Out::ModeChanged { mode, reason } => {
                    if mode == Mode::Display && self.info.is_none() {
                        return Err(invalid("display before Welcome"));
                    }
                    self.mode = mode;
                    self.status.mode = Some(mode);
                    self.status.reason = Some(reason);
                    if mode == Mode::Native {
                        self.native();
                    }
                    if self.info.is_some() {
                        self.input(Input::Mode {
                            surface: self.id,
                            mode: if mode == Mode::Native {
                                touchbar_core::Mode::Native
                            } else {
                                touchbar_core::Mode::Display
                            },
                        });
                    }
                    self.retry();
                }
                Out::Touch {
                    phase,
                    contact,
                    x,
                    y,
                } => {
                    if self.mode != Mode::Display || self.header.is_none() {
                        return Err(invalid("touch outside display"));
                    }
                    self.input(Input::Touch {
                        surface: self.id,
                        contact: ContactId(contact),
                        phase,
                        x: x as f32,
                        y: y as f32,
                    });
                }
                Out::KeyEvent { key, phase } => {
                    if self.info.is_none() {
                        return Err(invalid("key before Welcome"));
                    }
                    self.input(Input::Key {
                        source: KeySource::Surface(self.id),
                        code: key,
                        phase,
                    });
                }
                Out::FramePresented {
                    index,
                    frame,
                    commit_us,
                } => {
                    if self.mode == Mode::Native {
                        return Ok(());
                    }
                    let pending = self
                        .pending
                        .as_ref()
                        .filter(|p| p.index == index && p.frame == frame)
                        .ok_or_else(|| invalid("unexpected frame acknowledgement"))?;
                    let sent = pending
                        .sent
                        .ok_or_else(|| invalid("acknowledgement before submission"))?;
                    let generation = pending.generation;
                    let lease = self
                        .held
                        .get_mut(index as usize)
                        .and_then(Option::as_mut)
                        .filter(|p| p.frame == frame)
                        .ok_or_else(|| invalid("acknowledgement of free buffer"))?;
                    lease.presented = true;
                    self.pending = None;
                    Self::sample(&mut self.commit, f64::from(commit_us));
                    Self::sample(&mut self.ack, now.0.saturating_sub(sent.0) as f64);
                    self.status.commit_us = Timing::from(&self.commit);
                    self.status.ack_ms = Timing::from(&self.ack);
                    self.events
                        .push_back(ClientEvent::Surface(SurfaceEvent::Presented {
                            surface: self.id,
                            generation,
                        }));
                    self.retry();
                }
                Out::BufferReleased { index, frame } => {
                    if self.mode == Mode::Native {
                        return Ok(());
                    }
                    let lease = self
                        .held
                        .get_mut(index as usize)
                        .ok_or_else(|| invalid("release index outside swapchain"))?;
                    if !lease
                        .as_ref()
                        .is_some_and(|p| p.frame == frame && p.presented)
                    {
                        return Err(invalid("release of unpresented or unrelated buffer"));
                    }
                    *lease = None;
                    self.retry();
                }
                Out::Revoked(reason) => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!("touchbard revoked: {reason:?}"),
                    ));
                }
            },
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct Handle(Rc<RefCell<State>>);
impl Handle {
    pub fn info(&self) -> Option<SurfaceInfo> {
        self.0.borrow().info.clone()
    }
    pub fn prepare_frame(&self) -> io::Result<Option<TargetSlot>> {
        let mut s = self.0.borrow_mut();
        if s.closed {
            return Err(io::ErrorKind::NotConnected.into());
        }
        if s.mode != Mode::Display {
            return Ok(None);
        }
        if s.prepared.is_some() {
            return Err(invalid("frame already prepared"));
        }
        let Some(header) = s.header else {
            s.deferred = true;
            return Ok(None);
        };
        if s.pending.is_some() {
            s.deferred = true;
            return Ok(None);
        }
        let Some(index) = s.held.iter().position(Option::is_none) else {
            s.deferred = true;
            return Ok(None);
        };
        let index = u32::try_from(index).expect("three buffers");
        s.prepared = Some(index);
        s.deferred = false;
        Ok(Some(TargetSlot {
            header,
            buffers: s.buffers.take(),
            index,
            force_full: s.full,
        }))
    }
    /// No image changed: release only the local reservation, never a submitted lease.
    pub fn finish_unchanged(&self, index: u32) -> io::Result<()> {
        let mut s = self.0.borrow_mut();
        if s.closed
            || s.mode != Mode::Display
            || s.full
            || s.prepared != Some(index)
            || s.pending.is_some()
        {
            return Err(invalid(
                "unchanged frame has no initialized free reservation",
            ));
        }
        s.prepared = None;
        Ok(())
    }
    pub fn submit(
        &self,
        generation: Generation,
        index: u32,
        damage: &[wire::DamageRect],
    ) -> io::Result<()> {
        let mut s = self.0.borrow_mut();
        if s.mode != Mode::Display || s.prepared != Some(index) || s.pending.is_some() {
            return Err(invalid("frame submitted without free prepared buffer"));
        }
        let header = s.header.ok_or_else(|| invalid("no swapchain"))?;
        if damage.len() > 8
            || damage
                .iter()
                .any(|r| !r.fits(header.physical_width, header.physical_height))
        {
            return Err(invalid("invalid frame damage"));
        }
        let frame = s.next_frame;
        s.next_frame = frame
            .checked_add(1)
            .ok_or_else(|| invalid("frame counter exhausted"))?;
        if header.frame_damage && !s.full {
            for r in damage {
                s.enqueue(In::FrameDamage(*r))?;
            }
        }
        s.enqueue(In::FrameReady { index, frame })?;
        s.held[index as usize] = Some(Lease {
            frame,
            presented: false,
        });
        s.prepared = None;
        s.pending = Some(Pending {
            index,
            frame,
            generation,
            sent: None,
        });
        s.full = false;
        Ok(())
    }
    pub fn watch_keys(&self, keys: &[WatchKey]) -> io::Result<()> {
        let desired: BTreeSet<_> = keys.iter().copied().collect();
        let mut s = self.0.borrow_mut();
        let changes = s
            .watched
            .difference(&desired)
            .map(|key| (*key, false))
            .chain(desired.difference(&s.watched).map(|key| (*key, true)))
            .collect::<Vec<_>>();
        for (key, enabled) in changes {
            s.enqueue(In::WatchKey { key, enabled })?
        }
        s.watched = desired;
        Ok(())
    }
    pub fn send_key(&self, key: SystemKey, phase: KeyPhase) -> io::Result<()> {
        let mut s = self.0.borrow_mut();
        if phase == KeyPhase::Pressed && s.mode != Mode::Display {
            return Err(invalid("key press outside display"));
        }
        s.enqueue(In::Key { key, phase })
    }
    pub fn backlight(&self, limit: f32) -> io::Result<()> {
        if !limit.is_finite() || !(0.0..=1.0).contains(&limit) {
            return Err(invalid("invalid backlight limit"));
        }
        // The finite 0..=1 check above bounds the rounded result to 0..=65535.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let limit = (limit * 65535.0).round() as u16;
        self.0.borrow_mut().enqueue(In::Backlight { limit })
    }
    pub fn target_path(&self, path: touchbar_gl::TargetPath) {
        self.0.borrow_mut().status.path = Some(path.as_str().into());
    }
    pub fn close(&self) {
        let mut s = self.0.borrow_mut();
        s.closed = true;
        s.native();
        let _ = s.wake.try_send(());
    }
}
pub struct Client {
    fd: Async<OwnedFd>,
    handle: Handle,
    wake: async_channel::Receiver<()>,
}
impl Client {
    pub fn from_fd(fd: OwnedFd, id: SurfaceId) -> io::Result<Self> {
        let flags = rustix::fs::fcntl_getfl(&fd)?;
        rustix::fs::fcntl_setfl(&fd, flags | rustix::fs::OFlags::NONBLOCK)?;
        let fd = Async::new(fd)?;
        let (wake, recv) = async_channel::bounded(1);
        let mut state = State {
            id,
            info: None,
            mode: Mode::Native,
            status: Status {
                connected: true,
                ..Status::default()
            },
            events: VecDeque::new(),
            outgoing: VecDeque::new(),
            watched: BTreeSet::new(),
            header: None,
            buffers: None,
            held: Vec::new(),
            prepared: None,
            pending: None,
            next_frame: 1,
            full: true,
            deferred: false,
            closed: false,
            commit: VecDeque::new(),
            ack: VecDeque::new(),
            wake,
        };
        state.enqueue(In::Hello {
            version: wire::VERSION,
        })?;
        Ok(Self {
            fd,
            handle: Handle(Rc::new(RefCell::new(state))),
            wake: recv,
        })
    }
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }
    pub fn surface_id(&self) -> Option<SurfaceId> {
        let s = self.handle.0.borrow();
        s.info.as_ref().map(|_| s.id)
    }
    pub fn status(&self) -> Status {
        self.handle.0.borrow().status.clone()
    }
    fn fail(&self, error: &io::Error) {
        self.handle.close();
        let mut s = self.handle.0.borrow_mut();
        s.events.clear();
        s.outgoing.clear();
        s.status.connected = false;
        s.status.error = Some(error.to_string());
        let _ = rustix::net::shutdown(self.fd.get_ref(), rustix::net::Shutdown::Both);
    }
    pub fn poll(&mut self, now: Timestamp) -> io::Result<Option<ClientEvent>> {
        let result = self.poll_inner(now);
        if let Err(error) = &result {
            self.fail(error);
        }
        result
    }
    fn poll_inner(&mut self, now: Timestamp) -> io::Result<Option<ClientEvent>> {
        let mut s = self.handle.0.borrow_mut();
        if s.closed {
            return Err(io::ErrorKind::NotConnected.into());
        }
        for _ in 0..32 {
            let Some(m) = s.outgoing.front().copied() else {
                break;
            };
            match wire::send_session(self.fd.get_ref(), m) {
                Ok(()) => {
                    s.outgoing.pop_front();
                    if let In::FrameReady { frame, .. } = m
                        && let Some(p) = s.pending.as_mut().filter(|p| p.frame == frame)
                    {
                        p.sent = Some(now);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        if let Some(e) = s.events.pop_front() {
            return Ok(Some(e));
        }
        match wire::recv_daemon(self.fd.get_ref()) {
            Ok(packet) => s.packet(packet, now)?,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
        Ok(s.events.pop_front())
    }
    pub async fn next_event(&mut self, clock: &dyn Clock) -> io::Result<ClientEvent> {
        enum Ready {
            Packet(DaemonPacket),
            Wake,
        }
        loop {
            if let Some(e) = self.poll(clock.now())? {
                return Ok(e);
            }
            let write = !self.handle.0.borrow().outgoing.is_empty();
            let read = async {
                self.fd
                    .read_with(|fd| wire::recv_daemon(fd))
                    .await
                    .map(Ready::Packet)
            };
            let writable = async {
                if !write {
                    return std::future::pending().await;
                }
                self.fd
                    .write_with(|fd| {
                        let mut s = self.handle.0.borrow_mut();
                        let Some(message) = s.outgoing.front().copied() else {
                            return Ok(Ready::Wake);
                        };
                        wire::send_session(fd, message)?;
                        s.outgoing.pop_front();
                        if let In::FrameReady { frame, .. } = message
                            && let Some(p) = s.pending.as_mut().filter(|p| p.frame == frame)
                        {
                            p.sent = Some(clock.now());
                        }
                        Ok(Ready::Wake)
                    })
                    .await
            };
            let wake = async {
                let _ = self.wake.recv().await;
                Ok(Ready::Wake)
            };
            let result =
                futures_lite::future::or(read, futures_lite::future::or(writable, wake)).await;
            let result = result.and_then(|ready| {
                if let Ready::Packet(packet) = ready {
                    self.handle.0.borrow_mut().packet(packet, clock.now())?;
                }
                Ok(())
            });
            if let Err(error) = result {
                self.fail(&error);
                return Err(error);
            }
        }
    }
}
