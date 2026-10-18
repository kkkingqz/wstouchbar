//! Real v2 seqpacket/memfd peer for session tests, with strict frame ownership checks.
//! Held-buffer writes are checked at protocol/acknowledgement boundaries, not by a ticker.
use async_io::{Async, Timer};
use futures_lite::future::{or, pending};
use rustix::net::{self, AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::{self, File},
    io,
    os::{
        fd::OwnedFd,
        unix::fs::{FileExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use touchbar_model::{KeyPhase, WatchKey};
use touchbar_protocol::{
    self as wire, DaemonMessage as Out, DamageRect, MAX_FRAME_DAMAGE, Mode, ModeReason,
    SessionMessage as In, SwapchainHeader,
};
const MAX_BUFFER_BYTES: u64 = 16 * 1024 * 1024;
const HISTORY: usize = 4096;
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
#[derive(Clone, Copy, Debug)]
pub struct FakeTouchbarDisplay {
    pub header: SwapchainHeader,
    pub width_mm100: u32,
    pub height_mm100: u32,
}
impl FakeTouchbarDisplay {
    fn welcome(self) -> Out {
        Out::Welcome {
            caps: 1,
            width: self.header.logical_width,
            height: self.header.logical_height,
            width_mm100: self.width_mm100,
            height_mm100: self.height_mm100,
        }
    }
    fn validate(self) -> io::Result<()> {
        self.header.validate()?;
        self.welcome().encode()?;
        if self.width_mm100 == 0
            || self.height_mm100 == 0
            || self.header.buffer_size > MAX_BUFFER_BYTES
        {
            return Err(invalid(
                "fake display needs positive physical size and buffers at most 16 MiB",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FakeTouchbardEvent {
    Connected,
    Disconnected,
    Received(In),
    Presented { index: u32, frame: u64 },
    Released { index: u32, frame: u64 },
}
#[derive(Clone, Copy)]
struct Frame {
    index: u32,
    number: u64,
    commit_us: u32,
    release_delay: Duration,
}
struct Held {
    frame: u64,
    pixels: Vec<u8>,
}
struct Listener {
    socket: Async<OwnedFd>,
    path: PathBuf,
    identity: (u64, u64),
}
impl Drop for Listener {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.identity) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
pub struct FakeTouchbard {
    display: FakeTouchbarDisplay,
    listener: Option<Listener>,
    peer: Option<Async<OwnedFd>>,
    hello: bool,
    mode: Mode,
    buffers: Vec<File>,
    held: BTreeMap<u32, Held>,
    current: Option<Frame>,
    pending: Option<(Frame, Instant)>,
    releases: VecDeque<(Frame, Instant)>,
    last_frame: u64,
    damage: Vec<DamageRect>,
    present_delay: Duration,
    release_delay: Duration,
    watched: BTreeSet<WatchKey>,
    physical: BTreeSet<WatchKey>,
    received: VecDeque<In>,
    received_count: u64,
}
impl FakeTouchbard {
    fn new(display: FakeTouchbarDisplay) -> io::Result<Self> {
        display.validate()?;
        Ok(Self {
            display,
            listener: None,
            peer: None,
            hello: false,
            mode: Mode::Native,
            buffers: Vec::new(),
            held: BTreeMap::new(),
            current: None,
            pending: None,
            releases: VecDeque::new(),
            last_frame: 0,
            damage: Vec::new(),
            present_delay: Duration::ZERO,
            release_delay: Duration::ZERO,
            watched: BTreeSet::new(),
            physical: BTreeSet::new(),
            received: VecDeque::new(),
            received_count: 0,
        })
    }
    /// The client end is blocking; callers may wrap it in Async for a session driver.
    pub fn pair(display: FakeTouchbarDisplay) -> io::Result<(Self, OwnedFd)> {
        let mut fake = Self::new(display)?;
        let (server, client) = net::socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        fake.peer = Some(Async::new(server)?);
        Ok((fake, client))
    }
    /// One active client at a time; disconnect allows the next connection to handshake.
    pub fn bind(path: &Path, display: FakeTouchbarDisplay) -> io::Result<Self> {
        let mut fake = Self::new(display)?;
        let raw = net::socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
            None,
        )?;
        net::bind(&raw, &SocketAddrUnix::new(path)?)?;
        let m = fs::symlink_metadata(path)?;
        let listener = Listener {
            socket: Async::new(raw)?,
            path: path.into(),
            identity: (m.dev(), m.ino()),
        };
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        net::listen(listener.socket.get_ref(), 8)?;
        fake.listener = Some(listener);
        Ok(fake)
    }
    /// Delays apply to future accepted frames; an outstanding presentation keeps its timing.
    pub fn set_delays(&mut self, presentation: Duration, release: Duration) {
        self.present_delay = presentation;
        self.release_delay = release;
    }
    pub fn display(&self) -> FakeTouchbarDisplay {
        self.display
    }
    pub fn received_count(&self) -> u64 {
        self.received_count
    }
    /// Drain the bounded history (the most recent 4096 messages).
    pub fn take_received(&mut self) -> Vec<In> {
        self.received.drain(..).collect()
    }
    pub fn pixels(&self, index: u32) -> io::Result<Vec<u8>> {
        let file = self
            .buffers
            .get(usize::try_from(index).map_err(|_| invalid("invalid buffer index"))?)
            .ok_or_else(|| invalid("invalid buffer index"))?;
        let mut bytes = vec![
            0;
            usize::try_from(self.display.header.buffer_size)
                .map_err(|_| invalid("buffer size exceeds address space"))?
        ];
        file.read_exact_at(&mut bytes, 0)?;
        Ok(bytes)
    }
    fn verify_held(&self) -> io::Result<()> {
        for (&index, held) in &self.held {
            if self.pixels(index)? != held.pixels {
                return Err(invalid("session wrote to a held buffer"));
            }
        }
        Ok(())
    }
    fn raw_send(&self, message: Out) -> io::Result<()> {
        let peer = self.peer.as_ref().ok_or(io::ErrorKind::NotConnected)?;
        // A stalled scenario fails promptly instead of blocking the test runner.
        wire::send_daemon(peer.get_ref(), message)
    }
    fn clear_frames(&mut self) {
        self.held.clear();
        self.current = None;
        self.pending = None;
        self.releases.clear();
        self.damage.clear();
        self.last_frame = 0;
        self.buffers.clear();
    }
    fn announce_display(&mut self) -> io::Result<()> {
        self.clear_frames();
        for _ in 0..self.display.header.buffer_count {
            let fd = rustix::fs::memfd_create(
                "fake-touchbard",
                rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
            )?;
            let file = File::from(fd);
            file.set_len(self.display.header.buffer_size)?;
            rustix::fs::fcntl_add_seals(
                &file,
                rustix::fs::SealFlags::SHRINK
                    | rustix::fs::SealFlags::GROW
                    | rustix::fs::SealFlags::SEAL,
            )?;
            self.buffers.push(file);
        }
        let buffers = self
            .buffers
            .iter()
            .map(|f| f.try_clone().map(OwnedFd::from))
            .collect::<io::Result<Vec<_>>>()?;
        self.raw_send(self.display.welcome())?;
        wire::send_swapchain(
            self.peer
                .as_ref()
                .ok_or(io::ErrorKind::NotConnected)?
                .get_ref(),
            self.display.header,
            &buffers,
        )?;
        Ok(())
    }
    pub fn set_mode(&mut self, mode: Mode, reason: ModeReason) -> io::Result<()> {
        self.verify_held()?;
        let previous = self.mode;
        self.mode = mode;
        if self.hello {
            if mode == Mode::Display && previous != mode {
                self.announce_display()?;
            }
            if mode == Mode::Native {
                self.clear_frames();
            }
            self.raw_send(Out::ModeChanged { mode, reason })?;
        }
        Ok(())
    }
    /// Script desktop input/revocation. Frame acknowledgements are generated by next_event.
    pub fn send_message(&mut self, message: Out) -> io::Result<()> {
        self.verify_held()?;
        match message {
            Out::ModeChanged { mode, reason } => return self.set_mode(mode, reason),
            Out::Welcome { .. } | Out::FramePresented { .. } | Out::BufferReleased { .. } => {
                return Err(invalid(
                    "use display configuration and acknowledgement delays",
                ));
            }
            Out::KeyEvent { key, phase } => {
                if phase == KeyPhase::Pressed {
                    self.physical.insert(key);
                } else {
                    self.physical.remove(&key);
                }
            }
            _ => {}
        }
        self.raw_send(message)?;
        if matches!(message, Out::Revoked(_)) {
            self.mode = Mode::Native;
            self.disconnect();
        }
        Ok(())
    }
    /// Change physical state; unwatched keys do not generate protocol messages.
    pub fn set_physical_key(&mut self, key: WatchKey, phase: KeyPhase) -> io::Result<()> {
        if phase == KeyPhase::Pressed {
            self.physical.insert(key);
        } else {
            self.physical.remove(&key);
        }
        if self.hello && self.watched.contains(&key) {
            self.raw_send(Out::KeyEvent { key, phase })?;
        }
        Ok(())
    }
    pub fn disconnect(&mut self) {
        self.peer = None;
        self.hello = false;
        self.watched.clear();
        self.clear_frames();
    }
    fn receive(&mut self, message: In) -> io::Result<FakeTouchbardEvent> {
        self.verify_held()?;
        if !self.hello {
            if message
                != (In::Hello {
                    version: wire::VERSION,
                })
            {
                return Err(invalid("expected Hello with current protocol version"));
            }
            self.hello = true;
            self.raw_send(Out::ModeChanged {
                mode: Mode::Native,
                reason: ModeReason::Owner,
            })?;
            if self.mode == Mode::Display {
                self.announce_display()?;
                self.raw_send(Out::ModeChanged {
                    mode: Mode::Display,
                    reason: ModeReason::Owner,
                })?;
            }
        } else {
            match message {
                In::Hello { .. } => return Err(invalid("duplicate Hello")),
                In::FrameDamage(rect) => {
                    if self.mode != Mode::Display {
                        return Err(invalid("damage in native mode"));
                    }
                    if !self.display.header.frame_damage
                        || !rect.fits(
                            self.display.header.physical_width,
                            self.display.header.physical_height,
                        )
                        || self.damage.len() >= MAX_FRAME_DAMAGE
                    {
                        return Err(invalid("damage outside buffer or over budget"));
                    }
                    self.damage.push(rect);
                }
                In::FrameReady { index, frame } => {
                    if self.mode != Mode::Display {
                        return Err(invalid("frame in native mode"));
                    }
                    if self.pending.is_some() {
                        return Err(invalid("second unconfirmed frame"));
                    }
                    if self.held.contains_key(&index) {
                        return Err(invalid("frame uses a held buffer"));
                    }
                    if frame <= self.last_frame {
                        return Err(invalid("frame number is not increasing"));
                    }
                    let pixels = self.pixels(index)?;
                    self.held.insert(index, Held { frame, pixels });
                    self.pending = Some((
                        Frame {
                            index,
                            number: frame,
                            release_delay: self.release_delay,
                            commit_us: u32::try_from(self.present_delay.as_micros())
                                .unwrap_or(i32::MAX as u32)
                                .min(i32::MAX as u32),
                        },
                        Instant::now() + self.present_delay,
                    ));
                    self.last_frame = frame;
                    self.damage.clear();
                }
                In::Key {
                    phase: KeyPhase::Pressed,
                    ..
                } if self.mode == Mode::Native => {
                    return Err(invalid("key pressed in native mode"));
                }
                In::WatchKey { key, enabled } => {
                    if enabled {
                        self.watched.insert(key);
                    } else {
                        self.watched.remove(&key);
                    }
                    self.raw_send(Out::KeyEvent {
                        key,
                        phase: if enabled && self.physical.contains(&key) {
                            KeyPhase::Pressed
                        } else {
                            KeyPhase::Released
                        },
                    })?;
                }
                _ => {}
            }
        }
        self.received_count = self.received_count.saturating_add(1);
        if self.received.len() == HISTORY {
            self.received.pop_front();
        }
        self.received.push_back(message);
        Ok(FakeTouchbardEvent::Received(message))
    }
    fn deadline(&self) -> Option<Instant> {
        self.pending
            .map(|(_, at)| at)
            .into_iter()
            .chain(self.releases.front().map(|(_, at)| *at))
            .min()
    }
    fn due(&mut self) -> io::Result<Option<FakeTouchbardEvent>> {
        self.verify_held()?;
        let now = Instant::now();
        if let Some((frame, at)) = self.pending
            && at <= now
        {
            self.raw_send(Out::FramePresented {
                index: frame.index,
                frame: frame.number,
                commit_us: frame.commit_us,
            })?;
            self.pending = None;
            if let Some(old) = self.current.replace(frame) {
                self.releases.push_back((old, now + frame.release_delay));
                self.releases.make_contiguous().sort_by_key(|(_, at)| *at);
            }
            return Ok(Some(FakeTouchbardEvent::Presented {
                index: frame.index,
                frame: frame.number,
            }));
        }
        if let Some((frame, at)) = self.releases.front().copied()
            && at <= now
        {
            self.raw_send(Out::BufferReleased {
                index: frame.index,
                frame: frame.number,
            })?;
            self.releases.pop_front();
            let held = self
                .held
                .remove(&frame.index)
                .ok_or_else(|| invalid("release of unowned buffer"))?;
            if held.frame != frame.number {
                return Err(invalid("release frame mismatch"));
            }
            return Ok(Some(FakeTouchbardEvent::Released {
                index: frame.index,
                frame: frame.number,
            }));
        }
        Ok(None)
    }
    /// Wait on socket readiness or an actual scheduled acknowledgement; idle has no timer.
    pub async fn next_event(&mut self) -> io::Result<FakeTouchbardEvent> {
        if self.peer.is_none() {
            let listener = self.listener.as_ref().ok_or(io::ErrorKind::NotConnected)?;
            let peer = listener
                .socket
                .read_with(|fd| {
                    net::accept_with(fd, SocketFlags::CLOEXEC | SocketFlags::NONBLOCK)
                        .map_err(io::Error::from)
                })
                .await?;
            self.peer = Some(Async::new(peer)?);
            return Ok(FakeTouchbardEvent::Connected);
        }
        loop {
            if let Some(event) = self.due()? {
                return Ok(event);
            }
            enum Ready {
                Message(io::Result<In>),
                Deadline,
            }
            let at = self.deadline();
            let peer = self.peer.as_ref().ok_or(io::ErrorKind::NotConnected)?;
            let ready = or(
                async { Ready::Message(peer.read_with(|fd| wire::recv_session(fd)).await) },
                async {
                    if let Some(at) = at {
                        Timer::at(at).await;
                    } else {
                        pending::<()>().await;
                    }
                    Ready::Deadline
                },
            )
            .await;
            match ready {
                Ready::Deadline => {}
                Ready::Message(Ok(message)) => return self.receive(message),
                Ready::Message(Err(e))
                    if matches!(
                        e.kind(),
                        io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
                    ) =>
                {
                    self.disconnect();
                    return Ok(FakeTouchbardEvent::Disconnected);
                }
                Ready::Message(Err(e)) => return Err(e),
            }
        }
    }
}
