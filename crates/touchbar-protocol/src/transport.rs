//! One send/receive per SOCK_SEQPACKET record. All received rights are owned immediately.
use crate::{DaemonMessage, SessionMessage, SwapchainHeader, invalid};
use rustix::net::{
    self, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags,
};
use std::{
    io::{self, IoSlice, IoSliceMut},
    mem::MaybeUninit,
    os::fd::{AsFd, OwnedFd},
};
#[derive(Debug)]
pub enum DaemonPacket {
    Message(DaemonMessage),
    Swapchain {
        header: SwapchainHeader,
        buffers: Vec<OwnedFd>,
    },
}
fn send(fd: impl AsFd, bytes: &[u8], buffers: &[OwnedFd]) -> io::Result<()> {
    let borrowed: Vec<_> = buffers.iter().map(AsFd::as_fd).collect();
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(3))];
    let mut ancillary = SendAncillaryBuffer::new(&mut space);
    if !borrowed.is_empty() && !ancillary.push(SendAncillaryMessage::ScmRights(&borrowed)) {
        return Err(invalid("too many swapchain descriptors"));
    }
    let sent = net::sendmsg(
        fd,
        &[IoSlice::new(bytes)],
        &mut ancillary,
        SendFlags::NOSIGNAL,
    )?;
    if sent != bytes.len() {
        return Err(invalid("incomplete seqpacket send"));
    }
    Ok(())
}
fn receive(fd: impl AsFd) -> io::Result<(Vec<u8>, Vec<OwnedFd>)> {
    let mut bytes = [0; 48];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(3))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
    let result = net::recvmsg(
        fd,
        &mut [IoSliceMut::new(&mut bytes)],
        &mut ancillary,
        RecvFlags::CMSG_CLOEXEC,
    )?;
    // RecvAncillaryBuffer::drop drains even on errors below, closing every installed fd.
    let mut buffers = Vec::new();
    for message in ancillary.drain() {
        match message {
            RecvAncillaryMessage::ScmRights(rights) => buffers.extend(rights),
            _ => return Err(invalid("unexpected ancillary data")),
        }
    }
    if result
        .flags
        .intersects(net::ReturnFlags::TRUNC | net::ReturnFlags::CTRUNC)
    {
        return Err(invalid("truncated seqpacket or descriptors"));
    }
    if result.bytes == 0 {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
    }
    Ok((bytes[..result.bytes].to_vec(), buffers))
}
pub fn send_session(fd: impl AsFd, message: SessionMessage) -> io::Result<()> {
    send(fd, &message.encode()?, &[])
}
pub fn send_daemon(fd: impl AsFd, message: DaemonMessage) -> io::Result<()> {
    send(fd, &message.encode()?, &[])
}
pub fn send_swapchain(
    fd: impl AsFd,
    header: SwapchainHeader,
    buffers: &[OwnedFd],
) -> io::Result<()> {
    if buffers.len() != usize::from(header.buffer_count) {
        return Err(invalid("swapchain descriptor count mismatch"));
    }
    send(fd, &header.encode()?, buffers)
}
pub fn recv_session(fd: impl AsFd) -> io::Result<SessionMessage> {
    let (bytes, buffers) = receive(fd)?;
    if !buffers.is_empty() {
        return Err(invalid("session message cannot carry descriptors"));
    }
    SessionMessage::decode(&bytes)
}
pub fn recv_daemon(fd: impl AsFd) -> io::Result<DaemonPacket> {
    let (bytes, buffers) = receive(fd)?;
    if bytes.len() == 48 {
        let header = SwapchainHeader::decode(&bytes)?;
        if buffers.len() != usize::from(header.buffer_count) {
            return Err(invalid("swapchain descriptor count mismatch"));
        }
        Ok(DaemonPacket::Swapchain { header, buffers })
    } else {
        if !buffers.is_empty() {
            return Err(invalid("daemon message cannot carry descriptors"));
        }
        DaemonMessage::decode(&bytes).map(DaemonPacket::Message)
    }
}
