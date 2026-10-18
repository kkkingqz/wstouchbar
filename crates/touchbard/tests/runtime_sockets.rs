use std::{
    io::{Read, Write},
    os::fd::OwnedFd,
    os::unix::{fs::PermissionsExt, net::UnixStream},
    time::Duration,
};
use touchbar_protocol::{self as transport, SessionMessage, control};
use touchbard::runtime::sockets::{Incoming, Sockets};
fn connect(path: &std::path::Path) -> OwnedFd {
    use rustix::net::*;
    let fd = socket(AddressFamily::UNIX, SocketType::SEQPACKET, None).unwrap();
    rustix::net::connect(&fd, &SocketAddrUnix::new(path).unwrap()).unwrap();
    fd
}
#[test]
fn real_seqpacket_hello_peercred_and_partial_packet_disconnect() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut sockets = Sockets::bind(dir.path()).unwrap();
        assert_eq!(
            std::fs::metadata(dir.path().join("session.sock"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o666
        );
        let fd = connect(&dir.path().join("session.sock"));
        transport::send_session(&fd, SessionMessage::Hello { version: 2 }).unwrap();
        let (id, uid) = match sockets.next().await.unwrap() {
            Incoming::Session {
                connection,
                uid,
                message: SessionMessage::Hello { version: 2 },
            } => (connection, uid),
            _ => panic!("expected hello"),
        };
        assert_eq!(uid, rustix::process::getuid().as_raw());
        rustix::net::send(&fd, b"short", rustix::net::SendFlags::NOSIGNAL).unwrap();
        assert!(
            matches!(sockets.next().await.unwrap(), Incoming::Disconnected {connection} if connection==id)
        );
    });
}
#[test]
fn control_framing_is_bounded_and_slow_peer_does_not_block_others() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut sockets = Sockets::bind(dir.path()).unwrap();
        let mut bad = UnixStream::connect(dir.path().join("control.sock")).unwrap();
        bad.write_all(&u32::MAX.to_le_bytes()).unwrap();
        let mut slow = UnixStream::connect(dir.path().join("control.sock")).unwrap();
        slow.write_all(&[1]).unwrap();
        let mut client = UnixStream::connect(dir.path().join("control.sock")).unwrap();
        client
            .write_all(&control::encode(&control::Request::new(control::Command::Status)).unwrap())
            .unwrap();
        let incoming = futures_lite::future::or(sockets.next(), async {
            async_io::Timer::after(Duration::from_secs(1)).await;
            panic!("slow client blocked status")
        })
        .await
        .unwrap();
        let id = match incoming {
            Incoming::Control {
                connection,
                uid,
                request,
            } => {
                assert_eq!(uid, rustix::process::getuid().as_raw());
                assert_eq!(request.command, control::Command::Status);
                connection
            }
            _ => panic!("expected status"),
        };
        sockets
            .respond(id, control::Response::new(control::Reply::Ack))
            .unwrap();
        // Writes are driven inside next; a second ready request lets that future return.
        let fd = connect(&dir.path().join("session.sock"));
        transport::send_session(&fd, SessionMessage::Hello { version: 2 }).unwrap();
        let _ = sockets.next().await.unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let response: control::Response = control::read_frame(&mut client).unwrap();
        assert_eq!(response.reply, control::Reply::Ack);
        let mut bytes = [0];
        assert_eq!(bad.read(&mut bytes).unwrap(), 0);
    });
}
#[test]
fn refuses_foreign_paths_and_removes_only_own_sockets() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("session.sock"), "keep").unwrap();
    assert!(Sockets::bind(dir.path()).is_err());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("session.sock")).unwrap(),
        "keep"
    );
    std::fs::remove_file(dir.path().join("session.sock")).unwrap();
    let sockets = Sockets::bind(dir.path()).unwrap();
    std::fs::remove_file(dir.path().join("session.sock")).unwrap();
    std::fs::write(dir.path().join("session.sock"), "replacement").unwrap();
    drop(sockets);
    assert!(dir.path().join("session.sock").exists());
    assert!(!dir.path().join("control.sock").exists());
}

#[test]
fn unhandshaken_peer_expires_without_any_other_activity() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut sockets = Sockets::bind(dir.path()).unwrap();
        let _peer = connect(&dir.path().join("session.sock"));
        let started = std::time::Instant::now();
        let event = futures_lite::future::or(sockets.next(), async {
            async_io::Timer::after(Duration::from_millis(1300)).await;
            panic!("Hello deadline was not armed after accept")
        })
        .await
        .unwrap();
        assert!(matches!(event, Incoming::Disconnected { .. }));
        assert!(
            started.elapsed() < Duration::from_millis(1200),
            "deadline needed unrelated wakeup"
        );
    });
}

#[test]
fn busy_session_cannot_starve_control_or_another_hello() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut sockets = Sockets::bind(dir.path()).unwrap();
        let busy = connect(&dir.path().join("session.sock"));
        transport::send_session(&busy, SessionMessage::Hello { version: 2 }).unwrap();
        let _ = sockets.next().await.unwrap();
        for _ in 0..8 {
            transport::send_session(&busy, SessionMessage::Backlight { limit: 123 }).unwrap();
        }
        let second = connect(&dir.path().join("session.sock"));
        transport::send_session(&second, SessionMessage::Hello { version: 2 }).unwrap();
        let mut client = UnixStream::connect(dir.path().join("control.sock")).unwrap();
        client
            .write_all(&control::encode(&control::Request::new(control::Command::Status)).unwrap())
            .unwrap();
        let mut hello = false;
        let mut status = false;
        for _ in 0..4 {
            match sockets.next().await.unwrap() {
                Incoming::Session {
                    message: SessionMessage::Hello { .. },
                    ..
                } => hello = true,
                Incoming::Control { .. } => status = true,
                _ => {}
            }
        }
        assert!(hello && status, "busy oldest session starved other inputs");
    });
}

#[test]
fn unsupported_control_version_gets_a_versioned_error() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut sockets = Sockets::bind(dir.path()).unwrap();
        let mut client = UnixStream::connect(dir.path().join("control.sock")).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = control::Request::new(control::Command::Status);
        request.version = 999;
        client
            .write_all(&control::encode(&request).unwrap())
            .unwrap();
        let (tx, rx) = async_channel::bounded(1);
        let reader = std::thread::spawn(move || {
            tx.send_blocking(control::read_frame::<_, control::Response>(&mut client))
                .unwrap();
        });
        let result = futures_lite::future::or(
            async {
                loop {
                    let _ = sockets.next().await.unwrap();
                }
            },
            async { rx.recv().await.unwrap() },
        )
        .await;
        assert!(
            matches!(result,Ok(control::Response{version:control::VERSION,reply:control::Reply::Error(ref e)}) if e.contains("version")),
            "{result:?}"
        );
        reader.join().unwrap();
    });
}

#[test]
fn oversized_status_becomes_a_bounded_error_response() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut sockets = Sockets::bind(dir.path()).unwrap();
        let mut client = UnixStream::connect(dir.path().join("control.sock")).unwrap();
        client
            .write_all(&control::encode(&control::Request::new(control::Command::Status)).unwrap())
            .unwrap();
        let Incoming::Control { connection, .. } = sockets.next().await.unwrap() else {
            panic!("request")
        };
        sockets
            .respond(
                connection,
                control::Response::new(control::Reply::Status(
                    serde_json::json!({"large":"x".repeat(control::MAX_FRAME)}),
                )),
            )
            .unwrap();
        let (tx, rx) = async_channel::bounded(1);
        let reader = std::thread::spawn(move || {
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            tx.send_blocking(control::read_frame::<_, control::Response>(&mut client))
                .unwrap();
        });
        let response = futures_lite::future::or(
            async {
                loop {
                    let _ = sockets.next().await.unwrap();
                }
            },
            async { rx.recv().await.unwrap().unwrap() },
        )
        .await;
        assert!(matches!(response.reply, control::Reply::Error(_)));
        reader.join().unwrap();
    });
}
