//! Bounded, cancellation-safe local transports. No client can block the daemon loop.
use async_io::{Async, Timer};
use futures_lite::future::poll_fn;
use rustix::net::{self, AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    io,
    os::{
        fd::OwnedFd,
        unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};
use touchbar_protocol::{self as wire, DaemonMessage, SessionMessage, SwapchainHeader, control};
const MAX_SESSIONS: usize = 33;
const MAX_UNHANDSHAKEN: usize = 16;
const MAX_CONTROL: usize = 8;
struct Endpoint {
    fd: Async<OwnedFd>,
    path: PathBuf,
    identity: (u64, u64),
    retry: Option<Timer>,
}
impl Endpoint {
    fn bind(dir: &Path, name: &str, kind: SocketType) -> io::Result<Self> {
        let path = dir.join(name);
        match fs::symlink_metadata(&path) {
            Ok(m) => {
                if !m.file_type().is_socket() || m.uid() != rustix::process::geteuid().as_raw() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "refusing to replace foreign/non-socket path",
                    ));
                }
                let probe = net::socket_with(
                    AddressFamily::UNIX,
                    kind,
                    SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
                    None,
                )?;
                match net::connect(&probe, &SocketAddrUnix::new(&path)?) {
                    Err(rustix::io::Errno::CONNREFUSED) => fs::remove_file(&path)?,
                    _ => return Err(io::ErrorKind::AddrInUse.into()),
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let raw = net::socket_with(
            AddressFamily::UNIX,
            kind,
            SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
            None,
        )?;
        net::bind(&raw, &SocketAddrUnix::new(&path)?)?;
        // Guard is installed before any later fallible operation, including chmod/listen.
        let m = fs::symlink_metadata(&path)?;
        let endpoint = Self {
            fd: Async::new(raw)?,
            path,
            identity: (m.dev(), m.ino()),
            retry: None,
        };
        fs::set_permissions(&endpoint.path, fs::Permissions::from_mode(0o666))?;
        net::listen(endpoint.fd.get_ref(), 32)?;
        Ok(endpoint)
    }
    fn accept(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<OwnedFd>> {
        if let Some(timer) = &mut self.retry {
            if Pin::new(timer).poll(cx).is_pending() {
                return Poll::Pending;
            }
            self.retry = None;
        }
        loop {
            match net::accept_with(
                self.fd.get_ref(),
                SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
            ) {
                Ok(fd) => return Poll::Ready(Ok(fd)),
                Err(rustix::io::Errno::AGAIN) => match self.fd.poll_readable(cx) {
                    Poll::Ready(Ok(())) => continue,
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                },
                Err(e) => {
                    // FD pressure and other local accept failures must not kill a session.
                    self.retry = Some(Timer::after(Duration::from_millis(100)));
                    let _ = Pin::new(self.retry.as_mut().unwrap()).poll(cx);
                    return Poll::Ready(Err(e.into()));
                }
            }
        }
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.identity) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
struct Peer {
    fd: Async<OwnedFd>,
    uid: u32,
    hello: bool,
    writable: bool,
    deadline: Instant,
}
struct Control {
    fd: Async<OwnedFd>,
    uid: u32,
    bytes: Vec<u8>,
    target: usize,
    output: Option<Vec<u8>>,
    offset: usize,
    delivered: bool,
    deadline: Instant,
}
#[derive(Debug)]
pub enum Incoming {
    Session {
        connection: u64,
        uid: u32,
        message: SessionMessage,
    },
    Disconnected {
        connection: u64,
    },
    Writable {
        connection: u64,
    },
    Control {
        connection: u64,
        uid: u32,
        request: control::Request,
    },
    Diagnostic(String),
}
pub struct Sockets {
    session: Endpoint,
    control: Endpoint,
    peers: BTreeMap<u64, Peer>,
    controls: BTreeMap<u64, Control>,
    serial: u64,
    cursor: u64,
    prefer_control: bool,
    timer: Option<(Instant, Timer)>,
}
impl Sockets {
    pub fn bind(dir: &Path) -> io::Result<Self> {
        let m = fs::symlink_metadata(dir)?;
        if !dir.is_absolute()
            || !m.is_dir()
            || m.uid() != rustix::process::geteuid().as_raw()
            || m.mode() & 0o022 != 0
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok(Self {
            session: Endpoint::bind(dir, "session.sock", SocketType::SEQPACKET)?,
            control: Endpoint::bind(dir, "control.sock", SocketType::STREAM)?,
            peers: BTreeMap::new(),
            controls: BTreeMap::new(),
            serial: 0,
            cursor: 0,
            prefer_control: true,
            timer: None,
        })
    }
    pub fn close(&mut self, id: u64) {
        self.peers.remove(&id);
    }
    pub fn send(&mut self, id: u64, message: DaemonMessage) -> io::Result<()> {
        self.with_peer(id, |fd| wire::send_daemon(fd, message))
    }
    pub fn swapchain(
        &mut self,
        id: u64,
        header: SwapchainHeader,
        buffers: &[OwnedFd],
    ) -> io::Result<()> {
        self.with_peer(id, |fd| wire::send_swapchain(fd, header, buffers))
    }
    fn with_peer(
        &mut self,
        id: u64,
        send: impl FnOnce(&OwnedFd) -> io::Result<()>,
    ) -> io::Result<()> {
        let p = self.peers.get_mut(&id).ok_or(io::ErrorKind::NotConnected)?;
        let result = send(p.fd.get_ref());
        p.writable = result
            .as_ref()
            .is_err_and(|e| e.kind() == io::ErrorKind::WouldBlock);
        result
    }
    pub fn respond(&mut self, id: u64, response: control::Response) -> io::Result<()> {
        let p = self
            .controls
            .get_mut(&id)
            .ok_or(io::ErrorKind::NotConnected)?;
        p.output = Some(
            control::encode(&response)
                .or_else(|e| control::encode(&control::Response::error(e.to_string())))?,
        );
        p.bytes.clear();
        Ok(())
    }
    fn next_id(&mut self) -> io::Result<u64> {
        self.serial = self
            .serial
            .checked_add(1)
            .ok_or_else(|| io::Error::other("connection id exhausted"))?;
        Ok(self.serial)
    }
    pub async fn next(&mut self) -> io::Result<Incoming> {
        poll_fn(|cx| {
            let result = self.poll(cx);
            if result.is_pending() {
                let deadline = self
                    .peers
                    .values()
                    .filter(|p| !p.hello)
                    .map(|p| p.deadline)
                    .chain(self.controls.values().map(|p| p.deadline))
                    .min();
                if self.timer.as_ref().map(|(at, _)| *at) != deadline {
                    self.timer = deadline.map(|at| (at, Timer::at(at)));
                }
                if let Some((_, t)) = &mut self.timer
                    && Pin::new(t).poll(cx).is_ready()
                {
                    cx.waker().wake_by_ref();
                }
            }
            result
        })
        .await
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<Incoming>> {
        let now = Instant::now();
        // Finish responses before accepting/returning another input. At most 8 x 64KiB.
        self.controls.retain(|_, p| {
            if p.deadline <= now {
                return false;
            }
            if let Some(bytes) = &p.output {
                loop {
                    match net::send(p.fd.get_ref(), &bytes[p.offset..], net::SendFlags::NOSIGNAL) {
                        Ok(0) => return false,
                        Ok(n) => {
                            p.offset += n;
                            if p.offset == bytes.len() {
                                return false;
                            }
                        }
                        Err(rustix::io::Errno::AGAIN) => match p.fd.poll_writable(cx) {
                            Poll::Ready(Ok(())) => continue,
                            Poll::Ready(Err(_)) => return false,
                            Poll::Pending => break,
                        },
                        Err(_) => return false,
                    }
                }
            }
            true
        });
        // Accept at most the fixed capacity; excess unhandshaken clients are closed immediately.
        for session in [false, true] {
            let count = if session {
                self.peers.len()
            } else {
                self.controls.len()
            };
            let cap = if session { MAX_SESSIONS } else { MAX_CONTROL };
            if count < cap {
                let accepted = if session {
                    self.session.accept(cx)
                } else {
                    self.control.accept(cx)
                };
                match accepted {
                    Poll::Ready(Ok(raw)) => {
                        let result = (|| -> io::Result<()> {
                            let uid = net::sockopt::socket_peercred(&raw)?.uid.as_raw();
                            let fd = Async::new(raw)?;
                            let id = self.next_id()?;
                            if session {
                                if self.peers.values().filter(|p| !p.hello).count()
                                    < MAX_UNHANDSHAKEN
                                {
                                    self.peers.insert(
                                        id,
                                        Peer {
                                            fd,
                                            uid,
                                            hello: false,
                                            writable: false,
                                            deadline: now + Duration::from_secs(1),
                                        },
                                    );
                                }
                            } else {
                                self.controls.insert(
                                    id,
                                    Control {
                                        fd,
                                        uid,
                                        bytes: Vec::new(),
                                        target: 4,
                                        output: None,
                                        offset: 0,
                                        delivered: false,
                                        deadline: now + Duration::from_secs(5),
                                    },
                                );
                            }
                            Ok(())
                        })();
                        if let Err(e) = result {
                            return Poll::Ready(Ok(Incoming::Diagnostic(e.to_string())));
                        }
                        cx.waker().wake_by_ref();
                    }
                    Poll::Ready(Err(e)) => {
                        return Poll::Ready(Ok(Incoming::Diagnostic(format!("accept: {e}"))));
                    }
                    Poll::Pending => {}
                }
            }
        }
        if self.prefer_control
            && let Poll::Ready(result) = self.poll_controls(cx)
        {
            self.prefer_control = false;
            return Poll::Ready(result);
        }
        let mut disconnected = None;
        let ids = self
            .peers
            .keys()
            .filter(|id| **id > self.cursor)
            .chain(self.peers.keys().filter(|id| **id <= self.cursor))
            .copied()
            .collect::<Vec<_>>();
        for id in ids {
            let p = self.peers.get_mut(&id).expect("known peer");
            if !p.hello && p.deadline <= now {
                disconnected = Some(id);
                break;
            }
            if p.writable {
                match p.fd.poll_writable(cx) {
                    Poll::Ready(Ok(())) => {
                        p.writable = false;
                        self.cursor = id;
                        self.prefer_control = true;
                        return Poll::Ready(Ok(Incoming::Writable { connection: id }));
                    }
                    Poll::Ready(Err(_)) => {
                        disconnected = Some(id);
                        break;
                    }
                    Poll::Pending => {}
                }
            }
            loop {
                match wire::recv_session(p.fd.get_ref()) {
                    Ok(message) => {
                        if !p.hello && !matches!(message, SessionMessage::Hello { .. }) {
                            disconnected = Some(id);
                            break;
                        }
                        p.hello = true;
                        self.cursor = id;
                        self.prefer_control = true;
                        return Poll::Ready(Ok(Incoming::Session {
                            connection: id,
                            uid: p.uid,
                            message,
                        }));
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => match p.fd.poll_readable(cx)
                    {
                        Poll::Ready(Ok(())) => continue,
                        Poll::Ready(Err(_)) => {
                            disconnected = Some(id);
                            break;
                        }
                        Poll::Pending => break,
                    },
                    Err(_) => {
                        disconnected = Some(id);
                        break;
                    }
                }
            }
            if disconnected.is_some() {
                break;
            }
        }
        if let Some(id) = disconnected {
            self.peers.remove(&id);
            self.cursor = id;
            self.prefer_control = true;
            return Poll::Ready(Ok(Incoming::Disconnected { connection: id }));
        }
        let result = self.poll_controls(cx);
        if result.is_ready() {
            self.prefer_control = false;
        }
        result
    }
    fn poll_controls(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<Incoming>> {
        let mut bad = None;
        for (&id, p) in &mut self.controls {
            if p.delivered {
                continue;
            }
            loop {
                let remaining = p.target - p.bytes.len();
                let mut bytes = [0u8; 8192];
                let take = remaining.min(bytes.len());
                match rustix::io::read(p.fd.get_ref(), &mut bytes[..take]) {
                    Ok(0) => {
                        bad = Some(id);
                        break;
                    }
                    Ok(n) => {
                        p.bytes.extend_from_slice(&bytes[..n]);
                        if p.bytes.len() == p.target {
                            if p.target == 4 {
                                let len =
                                    u32::from_le_bytes(p.bytes[..4].try_into().unwrap()) as usize;
                                if len == 0 || len > control::MAX_FRAME {
                                    bad = Some(id);
                                    break;
                                }
                                p.target = 4 + len;
                            } else {
                                match serde_json::from_slice::<control::Request>(&p.bytes[4..]) {
                                    Ok(request) if request.validate().is_ok() => {
                                        p.delivered = true;
                                        return Poll::Ready(Ok(Incoming::Control {
                                            connection: id,
                                            uid: p.uid,
                                            request,
                                        }));
                                    }
                                    Ok(request) => {
                                        let error = request
                                            .validate()
                                            .expect_err("version rejected by guard");
                                        p.output = control::encode(&control::Response::error(
                                            error.to_string(),
                                        ))
                                        .ok();
                                        p.delivered = true;
                                        cx.waker().wake_by_ref();
                                        break;
                                    }
                                    _ => {
                                        bad = Some(id);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Err(rustix::io::Errno::AGAIN) => match p.fd.poll_readable(cx) {
                        Poll::Ready(Ok(())) => continue,
                        Poll::Ready(Err(_)) => {
                            bad = Some(id);
                            break;
                        }
                        Poll::Pending => break,
                    },
                    Err(_) => {
                        bad = Some(id);
                        break;
                    }
                }
            }
            if bad.is_some() {
                break;
            }
        }
        if let Some(id) = bad {
            self.controls.remove(&id);
            cx.waker().wake_by_ref();
        }
        Poll::Pending
    }
}
