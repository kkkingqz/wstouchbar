//! Bounded, cancellation-safe evdev collection. Virtual events are wakeups only.
use super::{Changes, DeviceSnapshot, InputKind, KeyBitmap, KeyState};
use crate::{
    device::FsMonitor,
    input::{EVENT_SIZE, decode_events},
};
use async_io::Async;
use rustix::{event::epoll, fd::OwnedFd};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    os::fd::{AsFd, BorrowedFd},
};
use touchbar_model::WatchKey;
const MAX_NODES: usize = 256;
#[derive(Clone, Copy, Default)]
pub struct Capabilities {
    pub keys: bool,
    pub lid: bool,
}
pub trait KeyQueries {
    fn query(
        &self,
        fd: BorrowedFd<'_>,
        caps: Capabilities,
    ) -> io::Result<(KeyBitmap, Option<bool>)>;
}
struct Node {
    fd: OwnedFd,
    kind: InputKind,
    caps: Capabilities,
}
pub struct Batch {
    pub changes: Changes,
    pub faults: Vec<(u64, io::Error)>,
    pub rescan: bool,
}
pub struct KeyReader<Q> {
    poll: Async<OwnedFd>,
    nodes: BTreeMap<u64, Node>,
    source: Q,
    state: KeyState,
    dirty: bool,
}
impl<Q: KeyQueries> KeyReader<Q> {
    pub fn new(source: Q) -> io::Result<Self> {
        Ok(Self {
            poll: Async::new(epoll::create(epoll::CreateFlags::CLOEXEC)?)?,
            nodes: BTreeMap::new(),
            source,
            state: KeyState::new(BTreeSet::new()),
            dirty: true,
        })
    }
    pub fn set_wanted(&mut self, keys: BTreeSet<WatchKey>) {
        self.state.set_wanted(keys);
        self.dirty = true;
    }
    /// ID zero is reserved for the directory watcher. FDs must be nonblocking.
    pub fn insert(
        &mut self,
        id: u64,
        fd: OwnedFd,
        kind: InputKind,
        caps: Capabilities,
    ) -> io::Result<()> {
        if id == 0 || self.nodes.contains_key(&id) || self.nodes.len() >= MAX_NODES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid or excessive evdev registration",
            ));
        }
        epoll::add(
            self.poll.get_ref(),
            &fd,
            epoll::EventData::new_u64(id),
            epoll::EventFlags::IN,
        )?;
        self.nodes.insert(id, Node { fd, kind, caps });
        self.dirty = true;
        Ok(())
    }
    pub fn remove(&mut self, id: u64) {
        if let Some(node) = self.nodes.remove(&id) {
            let _ = epoll::delete(self.poll.get_ref(), &node.fd);
            self.dirty = true;
        }
    }
    pub fn watch(&self, watcher: &FsMonitor) -> io::Result<()> {
        epoll::add(
            self.poll.get_ref(),
            watcher,
            epoll::EventData::new_u64(0),
            epoll::EventFlags::IN,
        )
        .map_err(Into::into)
    }
    pub fn unwatch(&self, watcher: &FsMonitor) {
        let _ = epoll::delete(self.poll.get_ref(), watcher);
    }
    fn collect(&mut self, ready: Vec<epoll::Event>) -> Batch {
        let mut faults = Vec::new();
        let mut active = BTreeSet::new();
        let mut rescan = false;
        for event in ready {
            let id = event.data.u64();
            if id == 0 {
                rescan = true;
                continue;
            }
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            let mut bytes = [0; EVENT_SIZE * 64];
            let result = loop {
                match rustix::io::read(&node.fd, &mut bytes) {
                    Err(rustix::io::Errno::INTR) => continue,
                    r => break r,
                }
            };
            let error = match result {
                Ok(0) => Some(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "evdev disconnected",
                )),
                Ok(n) => match decode_events(&bytes[..n]) {
                    Ok(events) => {
                        if events.iter().any(|e| matches!(e.kind, 1 | 2 | 3 | 5)) {
                            active.insert(id);
                        }
                        None
                    }
                    Err(e) => Some(e),
                },
                Err(rustix::io::Errno::AGAIN) => None,
                Err(e) => Some(e.into()),
            };
            if let Some(error) = error {
                faults.push((id, error));
                self.remove(id);
            }
        }
        let mut snapshots = Vec::new();
        let mut dead = Vec::new();
        for (&id, node) in &self.nodes {
            let result = if node.kind == InputKind::Physical {
                self.source.query(node.fd.as_fd(), node.caps)
            } else {
                Ok((KeyBitmap::default(), None))
            };
            match result {
                Ok((keys, lid)) => snapshots.push(DeviceSnapshot {
                    kind: node.kind,
                    keys,
                    lid,
                    activity: active.contains(&id),
                }),
                Err(e) => {
                    dead.push(id);
                    faults.push((id, e));
                }
            }
        }
        for id in dead {
            self.remove(id);
        }
        self.dirty = false;
        Batch {
            changes: self.state.refresh(&snapshots),
            faults,
            rescan,
        }
    }
    /// Query all physical bitmaps, also after virtual wakeups or SYN_DROPPED. No event
    /// code/value from a virtual keyboard is trusted. Once bytes are consumed no await
    /// occurs, so cancellation cannot discard an already-consumed key edge.
    pub fn refresh(&mut self) -> Batch {
        self.collect(Vec::new())
    }
    pub async fn next(&mut self) -> io::Result<Batch> {
        if self.dirty {
            return Ok(self.refresh());
        }
        let ready = self
            .poll
            .read_with(|fd| {
                let mut events = Vec::with_capacity(MAX_NODES + 1);
                loop {
                    match epoll::wait(
                        fd,
                        rustix::buffer::spare_capacity(&mut events),
                        Some(&rustix::event::Timespec {
                            tv_sec: 0,
                            tv_nsec: 0,
                        }),
                    ) {
                        Err(rustix::io::Errno::INTR) => continue,
                        Err(e) => return Err(e.into()),
                        Ok(_) => break,
                    }
                }
                if events.is_empty() {
                    return Err(io::ErrorKind::WouldBlock.into());
                }
                Ok(events)
            })
            .await?;
        Ok(self.collect(ready))
    }
}
