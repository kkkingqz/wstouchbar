use std::{
    fs,
    io::{Cursor, Write},
    os::unix::fs::{PermissionsExt, symlink},
    time::Duration,
};
use touchbar_control::*;
#[test]
fn framing_checks_version_length_truncation_and_one_complete_json_message() {
    let request = Request::new(Command::Status);
    let bytes = encode(&request).unwrap();
    assert_eq!(
        u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize,
        bytes.len() - 4
    );
    assert_eq!(
        read_frame::<_, Request>(&mut Cursor::new(&bytes)).unwrap(),
        request
    );
    for bytes in [
        vec![],
        vec![1, 2],
        0u32.to_le_bytes().to_vec(),
        u32::try_from(MAX_FRAME + 1).unwrap().to_le_bytes().to_vec(),
        bytes[..bytes.len() - 1].to_vec(),
    ] {
        assert!(read_frame::<_, Request>(&mut Cursor::new(bytes)).is_err());
    }
    let mut old = request;
    old.version += 1;
    assert!(old.validate().is_err());
}
#[test]
fn simulator_paths_and_state_are_isolated() {
    let runtime = std::path::Path::new("/runtime");
    assert_eq!(
        socket_path(runtime, false),
        runtime.join("touchbar/session-control.sock")
    );
    assert_eq!(
        socket_path(runtime, true),
        runtime.join("touchbar/simulator/session-control.sock")
    );
    assert_ne!(state_home(runtime, false), state_home(runtime, true));
}
#[test]
fn server_is_private_exclusive_and_cleans_only_its_socket() {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = socket_path(dir.path(), false);
    let server = Server::bind(dir.path(), false).unwrap();
    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(Server::bind(dir.path(), false).is_err());
    let simulator = Server::bind(dir.path(), true).unwrap();
    assert!(socket_path(dir.path(), true).exists());
    drop(simulator);
    drop(server);
    assert!(!path.exists());
    fs::write(&path, "preserve").unwrap();
    assert!(Server::bind(dir.path(), false).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"preserve");
    fs::remove_file(&path).unwrap();
    fs::remove_dir(path.parent().unwrap().join("simulator")).unwrap();
    fs::remove_dir(path.parent().unwrap()).unwrap();
    symlink("/tmp", path.parent().unwrap()).unwrap();
    assert!(Server::bind(dir.path(), false).is_err());
}
#[test]
fn incomplete_client_does_not_block_other_requests_and_bad_versions_get_errors() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let mut server = Server::bind(dir.path(), false).unwrap();
        let path = socket_path(dir.path(), false);
        let mut slow = std::os::unix::net::UnixStream::connect(&path).unwrap();
        slow.write_all(&[42]).unwrap();
        let client = async { call(&path, Command::Status).await.unwrap() };
        let serving = async {
            let incoming = server.next().await.unwrap();
            assert_eq!(incoming.request.command, Command::Status);
            server
                .respond(incoming, Reply::Status(serde_json::json!({"ok":true})))
                .unwrap();
            // Poll transport to flush the response, without requiring another request.
            futures_lite::future::or(
                async {
                    let _ = server.next().await;
                },
                async {
                    async_io::Timer::after(Duration::from_millis(100)).await;
                },
            )
            .await;
        };
        let reply = futures_lite::future::zip(client, serving).await.0;
        assert!(matches!(reply, Reply::Status(value) if value["ok"] == true));
    });
}

#[test]
fn peer_credentials_reject_another_uid_and_oversized_serialization_is_bounded() {
    let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
    let uid = rustix::process::getuid().as_raw();
    check_peer(&a, uid).unwrap();
    assert_eq!(
        check_peer(&a, uid.wrapping_add(1)).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(encode(&"x".repeat(MAX_FRAME)).is_err());
}
#[test]
fn old_version_gets_a_versioned_error_and_server_keeps_running() {
    futures_lite::future::block_on(async {
        use futures_lite::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let mut server = Server::bind(dir.path(), false).unwrap();
        let client = async {
            let mut stream = async_io::Async::<std::os::unix::net::UnixStream>::connect(
                socket_path(dir.path(), false),
            )
            .await
            .unwrap();
            stream
                .write_all(
                    &encode(&Request {
                        version: VERSION + 1,
                        command: Command::Status,
                    })
                    .unwrap(),
                )
                .await
                .unwrap();
            let mut prefix = [0; 4];
            stream.read_exact(&mut prefix).await.unwrap();
            let mut bytes = vec![0; usize::try_from(u32::from_le_bytes(prefix)).unwrap()];
            stream.read_exact(&mut bytes).await.unwrap();
            let response: Response = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(response.version, VERSION);
            assert!(matches!(response.reply, Reply::Error(e) if e.contains("version")));
        };
        futures_lite::future::or(
            async {
                let _ = server.next().await;
                panic!("invalid version reached handler");
            },
            client,
        )
        .await;
    });
}
