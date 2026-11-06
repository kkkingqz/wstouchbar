//! Session-side protocol client for full daemon tests. Every receive has a deadline.
use async_io::{Async, Timer};
use futures_lite::future::or;
use rustix::net::{self, AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
use std::{io, os::fd::OwnedFd, path::Path, time::Duration};
use touchbar_protocol::{DaemonPacket, SessionMessage};
pub struct TestSessionPeer {
    socket: Async<OwnedFd>,
}
impl TestSessionPeer {
    pub fn connect(path: &Path) -> io::Result<Self> {
        let fd = net::socket_with(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        net::connect(&fd, &SocketAddrUnix::new(path)?)?;
        Self::from_fd(fd)
    }
    pub fn from_fd(fd: OwnedFd) -> io::Result<Self> {
        Ok(Self {
            socket: Async::new(fd)?,
        })
    }
    pub fn send(&self, message: SessionMessage) -> io::Result<()> {
        touchbar_protocol::send_session(self.socket.get_ref(), message)
    }
    pub async fn receive(&self, timeout: Duration) -> io::Result<DaemonPacket> {
        or(
            self.socket
                .read_with(|fd| touchbar_protocol::recv_daemon(fd)),
            async {
                Timer::after(timeout).await;
                Err(io::ErrorKind::TimedOut.into())
            },
        )
        .await
    }
}
