use rustix::{
    io::{FdFlags, fcntl_getfd},
    net::{self, AddressFamily, SendFlags, SocketFlags, SocketType},
};
use std::{
    io::IoSlice,
    os::fd::{AsFd, OwnedFd},
};
use touchbar_protocol::*;
fn pair() -> (OwnedFd, OwnedFd) {
    net::socketpair(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap()
}
fn header() -> SwapchainHeader {
    SwapchainHeader {
        logical_width: 20,
        logical_height: 4,
        physical_width: 20,
        physical_height: 4,
        format: XRGB8888,
        pitch: 80,
        buffer_size: 320,
        buffer_count: 2,
        frame_damage: true,
    }
}
fn raw(fd: &OwnedFd, bytes: &[u8], fds: &[OwnedFd]) {
    let borrowed: Vec<_> = fds.iter().map(AsFd::as_fd).collect();
    let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(8))];
    let mut ancillary = net::SendAncillaryBuffer::new(&mut space);
    if !fds.is_empty() {
        assert!(ancillary.push(net::SendAncillaryMessage::ScmRights(&borrowed)));
    }
    net::sendmsg(
        fd,
        &[IoSlice::new(bytes)],
        &mut ancillary,
        SendFlags::NOSIGNAL,
    )
    .unwrap();
}
fn count_target(fd: &OwnedFd) -> usize {
    use std::os::fd::AsRawFd;
    let target = std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).unwrap();
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| std::fs::read_link(entry.path()).is_ok_and(|value| value == target))
        .count()
}
fn fds(n: usize) -> Vec<OwnedFd> {
    (0..n)
        .map(|_| std::os::unix::net::UnixDatagram::unbound().unwrap().into())
        .collect()
}
#[test]
fn seqpacket_roundtrip_and_swapchain_descriptors_are_cloexec() {
    let (a, b) = pair();
    let hello = SessionMessage::Hello { version: 2 };
    send_session(&a, hello).unwrap();
    assert_eq!(recv_session(&b).unwrap(), hello);
    let message = DaemonMessage::FramePresented {
        index: 1,
        frame: 42,
        commit_us: 123,
    };
    send_daemon(&b, message).unwrap();
    assert!(matches!(recv_daemon(&a).unwrap(), DaemonPacket::Message(v) if v == message));
    let handles = fds(2);
    send_swapchain(&b, header(), &handles).unwrap();
    let DaemonPacket::Swapchain {
        header: got,
        buffers,
    } = recv_daemon(&a).unwrap()
    else {
        panic!("expected swapchain")
    };
    assert_eq!(got, header());
    assert_eq!(buffers.len(), 2);
    for fd in buffers {
        assert!(fcntl_getfd(fd).unwrap().contains(FdFlags::CLOEXEC));
    }
    assert!(send_swapchain(&b, header(), &fds(3)).is_err());
    let triple = SwapchainHeader {
        buffer_count: 3,
        ..header()
    };
    send_swapchain(&b, triple, &fds(3)).unwrap();
    let DaemonPacket::Swapchain {
        header: got,
        buffers,
    } = recv_daemon(&a).unwrap()
    else {
        panic!("expected triple swapchain")
    };
    assert_eq!(got, triple);
    assert_eq!(buffers.len(), 3);
    for fd in buffers {
        assert!(fcntl_getfd(fd).unwrap().contains(FdFlags::CLOEXEC));
    }
}
#[test]
fn malformed_records_and_rights_are_rejected_without_partial_result() {
    for count in [0, 1, 3, 8] {
        let (a, b) = pair();
        let handles = fds(count);
        raw(&a, &header().encode().unwrap(), &handles);
        assert!(recv_daemon(&b).is_err(), "fd count {count}");
        for fd in &handles {
            assert_eq!(count_target(fd), 1, "received rights leaked");
        }
    }
    for bytes in [vec![0; 31], vec![0; 33], vec![0; 49], vec![0; 4096]] {
        let (a, b) = pair();
        raw(&a, &bytes, &[]);
        assert!(recv_daemon(&b).is_err());
    }
    let (a, b) = pair();
    raw(
        &a,
        &SessionMessage::Hello { version: 2 }.encode().unwrap(),
        &fds(1),
    );
    assert!(recv_session(&b).is_err());
    raw(
        &a,
        &DaemonMessage::BufferReleased { index: 0, frame: 0 }
            .encode()
            .unwrap(),
        &fds(1),
    );
    assert!(recv_daemon(&b).is_err());
    raw(&a, &header().encode().unwrap(), &fds(2));
    assert!(recv_session(&b).is_err());
}
