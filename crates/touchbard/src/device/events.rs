//! Kernel uevent and filesystem notifications; no timers or background polling.
use async_io::Async;
use rustix::{
    fd::OwnedFd,
    fs::inotify,
    net::{self, AddressFamily, RecvFlags, SocketFlags, SocketType, netlink},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    io,
    mem::MaybeUninit,
    path::{Component, Path, PathBuf},
};
const MAX_UEVENT: usize = 65536;
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uevent {
    pub action: String,
    pub devpath: PathBuf,
    pub subsystem: String,
}
impl Uevent {
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() > MAX_UEVENT || bytes.last() != Some(&0) {
            return Err(invalid("incomplete or oversized uevent"));
        }
        let mut fields = bytes
            .split(|byte| *byte == 0)
            .filter(|field| !field.is_empty());
        let header = std::str::from_utf8(
            fields
                .next()
                .ok_or_else(|| invalid("missing uevent header"))?,
        )
        .map_err(|_| invalid("uevent is not UTF-8"))?;
        let (action, devpath) = header
            .split_once('@')
            .ok_or_else(|| invalid("invalid uevent header"))?;
        if !matches!(
            action,
            "add" | "remove" | "change" | "move" | "online" | "offline" | "bind" | "unbind"
        ) {
            return Err(invalid("unknown uevent action"));
        }
        let path = Path::new(devpath);
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(invalid("noncanonical uevent devpath"));
        }
        let mut values = BTreeMap::new();
        for field in fields {
            let field =
                std::str::from_utf8(field).map_err(|_| invalid("uevent field is not UTF-8"))?;
            let (key, value) = field
                .split_once('=')
                .ok_or_else(|| invalid("invalid uevent field"))?;
            if values.insert(key, value).is_some() {
                return Err(invalid("duplicate uevent field"));
            }
        }
        if values.get("ACTION") != Some(&action) || values.get("DEVPATH") != Some(&devpath) {
            return Err(invalid("uevent header disagrees with fields"));
        }
        let subsystem = values
            .get("SUBSYSTEM")
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 128
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
            .ok_or_else(|| invalid("missing or invalid subsystem"))?;
        Ok(Self {
            action: action.into(),
            devpath: path.into(),
            subsystem: (*subsystem).into(),
        })
    }
    pub fn from_kernel(sender_port: u32, bytes: &[u8]) -> io::Result<Option<Self>> {
        if sender_port != 0 {
            return Ok(None);
        }
        Self::decode(bytes).map(Some)
    }
    pub fn relevant(&self) -> bool {
        self.devpath.starts_with("/devices")
            && matches!(
                self.subsystem.as_str(),
                "usb" | "hid" | "drm" | "input" | "backlight"
            )
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KernelNotice {
    Event(Uevent),
    Overflow,
}
pub struct KernelMonitor {
    socket: Async<OwnedFd>,
}
impl KernelMonitor {
    pub fn new() -> io::Result<Self> {
        let socket = net::socket_with(
            AddressFamily::NETLINK,
            SocketType::DGRAM,
            SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            Some(netlink::KOBJECT_UEVENT),
        )?;
        net::bind(&socket, &netlink::SocketAddrNetlink::new(0, 1))?;
        Ok(Self {
            socket: Async::new(socket)?,
        })
    }
    pub async fn next(&self) -> io::Result<KernelNotice> {
        loop {
            let notice = self
                .socket
                .read_with(|fd| {
                    let mut bytes = [0; MAX_UEVENT];
                    let (copied, total, address) =
                        match net::recvfrom(fd, &mut bytes[..], RecvFlags::TRUNC) {
                            Ok(value) => value,
                            Err(rustix::io::Errno::NOBUFS) => {
                                return Ok(Some(KernelNotice::Overflow));
                            }
                            Err(e) => return Err(e.into()),
                        };
                    let Some(address) =
                        address.and_then(|addr| netlink::SocketAddrNetlink::try_from(addr).ok())
                    else {
                        return Ok(None);
                    };
                    // Netlink port IDs are assigned by the kernel; userspace cannot send as port zero.
                    if address.pid() != 0 {
                        return Ok(None);
                    }
                    if copied != total || total > MAX_UEVENT {
                        return Err(invalid("truncated kernel uevent"));
                    }
                    Ok(Uevent::from_kernel(address.pid(), &bytes[..copied])?
                        .filter(Uevent::relevant)
                        .map(KernelNotice::Event))
                })
                .await?;
            if let Some(notice) = notice {
                return Ok(notice);
            }
        }
    }
}
/// Recreate after a changed snapshot so a missing target can gain a more specific watch.
pub struct FsMonitor {
    socket: Async<OwnedFd>,
    filters: BTreeMap<i32, Option<BTreeSet<OsString>>>,
}
impl FsMonitor {
    pub fn new(paths: &[PathBuf]) -> io::Result<Self> {
        let fd = inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)?;
        let mut filters: BTreeMap<i32, Option<BTreeSet<OsString>>> = BTreeMap::new();
        for target in paths {
            let mut parent = target.as_path();
            while !parent.exists() {
                parent = parent
                    .parent()
                    .ok_or_else(|| invalid("watch path has no existing ancestor"))?;
            }
            let filter = if parent == target {
                None
            } else {
                Some(
                    target
                        .strip_prefix(parent)
                        .expect("ancestor")
                        .components()
                        .next()
                        .expect("missing component")
                        .as_os_str()
                        .to_owned(),
                )
            };
            let wd = inotify::add_watch(
                &fd,
                parent,
                inotify::WatchFlags::CREATE
                    | inotify::WatchFlags::DELETE
                    | inotify::WatchFlags::MODIFY
                    | inotify::WatchFlags::CLOSE_WRITE
                    | inotify::WatchFlags::ATTRIB
                    | inotify::WatchFlags::MOVED_FROM
                    | inotify::WatchFlags::MOVED_TO
                    | inotify::WatchFlags::DELETE_SELF
                    | inotify::WatchFlags::MOVE_SELF,
            )?;
            let entry = filters.entry(wd).or_insert_with(|| Some(BTreeSet::new()));
            if let Some(name) = filter {
                if let Some(names) = entry {
                    names.insert(name);
                }
            } else {
                *entry = None;
            }
        }
        Ok(Self {
            socket: Async::new(fd)?,
            filters,
        })
    }
    /// Consume one bounded kernel batch. False means only unrelated ancestor entries changed.
    pub fn try_changed(&self) -> io::Result<bool> {
        Self::read_batch(self.socket.get_ref(), &self.filters)
    }
    fn read_batch(
        fd: &OwnedFd,
        filters: &BTreeMap<i32, Option<BTreeSet<OsString>>>,
    ) -> io::Result<bool> {
        let mut space = [MaybeUninit::uninit(); 8192];
        let mut reader = inotify::Reader::new(fd, &mut space);
        let mut changed = false;
        loop {
            let event = reader.next()?;
            if event.events().intersects(
                inotify::ReadFlags::QUEUE_OVERFLOW
                    | inotify::ReadFlags::IGNORED
                    | inotify::ReadFlags::DELETE_SELF
                    | inotify::ReadFlags::MOVE_SELF,
            ) {
                changed = true;
            }
            if let Some(filter) = filters.get(&event.wd()) {
                changed |= filter.as_ref().is_none_or(|names| {
                    event.file_name().is_some_and(|name| {
                        names
                            .iter()
                            .any(|candidate| candidate.as_encoded_bytes() == name.to_bytes())
                    })
                });
            }
            if reader.is_buffer_empty() {
                return Ok(changed);
            }
        }
    }
    pub async fn next(&self) -> io::Result<()> {
        loop {
            if self
                .socket
                .read_with(|fd| Self::read_batch(fd, &self.filters))
                .await?
            {
                return Ok(());
            }
        }
    }
}
