use super::{Client, ClientEvent, Handle, Status};
use crate::Clock;
use async_io::Async;
use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};
use rustix::net::{self, AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
use std::{
    ffi::OsString,
    io,
    os::fd::AsFd,
    path::{Path, PathBuf},
};
use touchbar_model::{SurfaceId, Timestamp};
#[derive(Debug)]
pub enum ConnectionEvent {
    Protocol(ClientEvent),
    Disconnected {
        surface: Option<SurfaceId>,
        error: String,
    },
}
struct Watcher {
    inotify: Inotify,
    ready: Async<std::os::fd::OwnedFd>,
    watch: Option<WatchDescriptor>,
    name: OsString,
    path: PathBuf,
}
impl Watcher {
    fn new(path: PathBuf) -> io::Result<Self> {
        let inotify = Inotify::init()?;
        let ready = Async::new(rustix::io::dup(inotify.as_fd())?)?;
        let mut this = Self {
            inotify,
            ready,
            watch: None,
            name: OsString::new(),
            path,
        };
        this.rearm()?;
        Ok(this)
    }
    fn rearm(&mut self) -> io::Result<()> {
        let mut directory = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("socket has no parent"))?;
        let mut name = self
            .path
            .file_name()
            .ok_or_else(|| io::Error::other("socket has no filename"))?;
        while !directory.is_dir() {
            name = directory
                .file_name()
                .ok_or_else(|| io::Error::other("socket has no existing ancestor"))?;
            directory = directory
                .parent()
                .ok_or_else(|| io::Error::other("socket has no existing ancestor"))?;
        }
        let new = self.inotify.watches().add(
            directory,
            WatchMask::CREATE
                | WatchMask::MOVED_TO
                | WatchMask::DELETE
                | WatchMask::MOVED_FROM
                | WatchMask::DELETE_SELF
                | WatchMask::MOVE_SELF
                | WatchMask::ATTRIB,
        )?;
        if let Some(old) = self.watch.replace(new.clone())
            && old != new
        {
            let _ = self.inotify.watches().remove(old);
        }
        self.name = name.to_owned();
        Ok(())
    }
    fn read(
        inotify: &mut Inotify,
        watch: &Option<WatchDescriptor>,
        name: &OsString,
    ) -> io::Result<bool> {
        let mut changed = false;
        let mut bytes = [0; 8192];
        for _ in 0..8 {
            match inotify.read_events(&mut bytes) {
                Ok(events) => {
                    for e in events {
                        if e.mask.contains(EventMask::Q_OVERFLOW)
                            || watch.as_ref() == Some(&e.wd)
                                && (e.name == Some(name.as_os_str())
                                    || e.mask.intersects(
                                        EventMask::DELETE_SELF
                                            | EventMask::MOVE_SELF
                                            | EventMask::IGNORED,
                                    ))
                        {
                            changed = true
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        Ok(changed)
    }
    fn drain(&mut self) -> io::Result<bool> {
        let changed = Self::read(&mut self.inotify, &self.watch, &self.name)?;
        if changed {
            self.rearm()?;
        }
        Ok(changed)
    }
    async fn changed(&mut self) -> io::Result<()> {
        let Self {
            ready,
            inotify,
            watch,
            name,
            ..
        } = self;
        ready
            .read_with(|_| {
                if Self::read(inotify, watch, name)? {
                    Ok(())
                } else {
                    Err(io::ErrorKind::WouldBlock.into())
                }
            })
            .await?;
        self.rearm()
    }
}
pub struct Connector {
    path: PathBuf,
    watcher: Watcher,
    client: Option<Client>,
    last: Status,
    retry: Option<Timestamp>,
    failures: u8,
    next_id: u64,
    connected_at: Option<Timestamp>,
}
impl Connector {
    pub fn new(path: PathBuf) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "touchbard socket must be absolute",
            ));
        }
        Ok(Self {
            watcher: Watcher::new(path.clone())?,
            path,
            client: None,
            last: Status::default(),
            retry: Some(Timestamp(0)),
            failures: 0,
            next_id: 10000,
            connected_at: None,
        })
    }
    pub fn handle(&self) -> Option<Handle> {
        self.client.as_ref().map(Client::handle)
    }
    pub fn status(&self) -> Status {
        self.client
            .as_ref()
            .map_or_else(|| self.last.clone(), Client::status)
    }
    fn backoff(&mut self, now: Timestamp) {
        let delay = (100u64 << self.failures.min(6)).min(5000);
        self.failures = self.failures.saturating_add(1);
        self.retry = Some(now.after(delay));
    }
    fn connect(&mut self, now: Timestamp) -> io::Result<()> {
        // Watch first, then connect: an appearance between the two cannot be missed.
        self.watcher.rearm()?;
        let attempt = || -> io::Result<Client> {
            let fd = net::socket_with(
                AddressFamily::UNIX,
                SocketType::SEQPACKET,
                SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
                None,
            )?;
            net::connect(&fd, &SocketAddrUnix::new(&self.path)?)?;
            Client::from_fd(fd, SurfaceId(self.next_id))
        };
        match attempt() {
            Ok(client) => {
                self.next_id = self
                    .next_id
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("surface ID exhausted"))?;
                self.client = Some(client);
                self.retry = None;
                self.connected_at = Some(now);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.last.error = Some(e.to_string());
                self.retry = None;
            }
            Err(e) => {
                self.last.error = Some(e.to_string());
                self.backoff(now);
            }
        }
        Ok(())
    }
    fn disconnected(&mut self, error: io::Error, now: Timestamp) -> ConnectionEvent {
        let old = self.client.take().expect("connected client");
        let surface = old.surface_id();
        self.last = old.status();
        self.last.connected = false;
        self.last.path = None;
        self.last.error = Some(error.to_string());
        old.handle().close();
        drop(old);
        if self
            .connected_at
            .take()
            .is_some_and(|at| now.0.saturating_sub(at.0) >= 1000)
        {
            self.failures = 0;
        }
        self.backoff(now);
        ConnectionEvent::Disconnected {
            surface,
            error: error.to_string(),
        }
    }
    pub async fn next_event(&mut self, clock: &dyn Clock) -> io::Result<ConnectionEvent> {
        loop {
            if let Some(client) = self.client.as_mut() {
                return match client.next_event(clock).await {
                    Ok(event) => Ok(ConnectionEvent::Protocol(event)),
                    Err(e) => Ok(self.disconnected(e, clock.now())),
                };
            }
            let now = clock.now();
            if self.watcher.drain()? {
                self.retry = Some(now);
            }
            if self.retry.is_some_and(|at| at <= now) {
                self.connect(now)?;
                if self.client.is_some() {
                    continue;
                }
            }
            let retry = self.retry;
            let watch = async { self.watcher.changed().await };
            let timer = async {
                if let Some(at) = retry {
                    async_io::Timer::after(std::time::Duration::from_millis(
                        at.0.saturating_sub(clock.now().0),
                    ))
                    .await;
                } else {
                    std::future::pending::<()>().await;
                }
                Ok(())
            };
            futures_lite::future::or(watch, timer).await?;
            self.retry = Some(clock.now());
        }
    }
    pub fn close(&mut self) {
        if let Some(client) = self.client.take() {
            client.handle().close();
        }
        self.last.connected = false;
        self.last.path = None;
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
