use std::time::Duration;
use touchbar_protocol::{self as wire, DaemonMessage, DaemonPacket, Mode, ModeReason};
use touchbar_testkit::TestSessionPeer;
#[test]
fn session_peer_preserves_protocol_and_bounds_silent_peers() {
    futures_lite::future::block_on(async {
        let (server, client) = rustix::net::socketpair(
            rustix::net::AddressFamily::UNIX,
            rustix::net::SocketType::SEQPACKET,
            rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        let peer = TestSessionPeer::from_fd(client).unwrap();
        assert_eq!(
            peer.receive(Duration::from_millis(5))
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::TimedOut
        );
        let message = DaemonMessage::ModeChanged {
            mode: Mode::Native,
            reason: ModeReason::Owner,
        };
        wire::send_daemon(&server, message).unwrap();
        assert!(
            matches!(peer.receive(Duration::from_secs(1)).await.unwrap(), DaemonPacket::Message(m) if m == message)
        );
        peer.send(wire::SessionMessage::Hello {
            version: wire::VERSION,
        })
        .unwrap();
        assert_eq!(
            wire::recv_session(&server).unwrap(),
            wire::SessionMessage::Hello {
                version: wire::VERSION
            }
        );
        drop(server);
        assert!(peer.receive(Duration::from_secs(1)).await.is_err());
    });
}
