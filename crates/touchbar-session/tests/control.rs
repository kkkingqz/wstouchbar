use std::{fs, os::unix::fs::PermissionsExt, rc::Rc};
use touchbar_control::{Command, Reply, Server, call, socket_path};
use touchbar_loader::*;
use touchbar_session::*;
#[test]
fn real_socket_status_dump_and_reload_use_session_and_retain_rejected_issues() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let desktop = Rc::new(touchbar_desktop::NullBackend::default());
        let (effects, _) = touchbar_effects::Engine::fake(desktop.clone());
        let mut session = Session::new(
            SessionOptions::new(dir.path().into(), UserConfig::default()),
            ManualClock::default(),
            Box::<touchbar_logic_runtime::MemoryStore>::default(),
            desktop,
            effects,
        )
        .unwrap();
        let request = LoadRequest {
            paths: vec![dir.path().join("plugins")],
            config_file: dir.path().join("config.toml"),
            bindings_file: dir.path().join("bindings.toml"),
        };
        assert!(
            session
                .load(prepare(request.clone(), session.compiler()))
                .accepted
        );
        session.pump().unwrap();
        let old = session.catalog().clone();
        fs::write(&request.config_file, "log='['\n").unwrap();
        let mut server = Server::bind(dir.path(), false).unwrap();
        let path = socket_path(dir.path(), false);
        let client = async {
            assert!(matches!(
                call(&path, Command::Reload).await.unwrap(),
                Reply::Ack
            ));
            let Reply::Status(status) = call(&path, Command::Status).await.unwrap() else {
                panic!("status response")
            };
            assert!(!status["load_issues"].as_array().unwrap().is_empty());
            assert!(status["metrics"]["frame_ms"]["p99"].is_null());
            let Reply::Dump(path) = call(&path, Command::Dump).await.unwrap() else {
                panic!("dump response")
            };
            let dump: touchbar_core::ReplayDump =
                serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            dump.replay().unwrap();
        };
        let serving = async {
            loop {
                let incoming = server.next().await.unwrap();
                let (reply, reload) = session.control(incoming.request.command.clone(), None);
                if reload {
                    assert!(
                        !session
                            .load(prepare(request.clone(), session.compiler()))
                            .accepted
                    );
                    assert_eq!(session.catalog(), &old);
                }
                server.respond(incoming, reply).unwrap();
            }
        };
        futures_lite::future::or(client, serving).await;
    });
}
#[test]
fn sim_commands_require_fake_and_validate_reserved_facts() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = Rc::new(touchbar_desktop::NullBackend::default());
    let (effects, _) = touchbar_effects::Engine::fake(desktop.clone());
    let mut session = Session::new(
        SessionOptions::new(dir.path().into(), UserConfig::default()),
        ManualClock::default(),
        Box::<touchbar_logic_runtime::MemoryStore>::default(),
        desktop,
        effects,
    )
    .unwrap();
    let command = Command::SimTheme {
        theme: touchbar_control::Theme::Light,
    };
    assert!(matches!(
        session.control(command.clone(), None).0,
        Reply::Error(_)
    ));
    let fake = touchbar_desktop::FakeBackend::default();
    assert!(matches!(
        session.control(command, Some(&fake)).0,
        Reply::Ack
    ));
    assert_eq!(
        fake.fact(&"appearance.scheme".into()),
        Some(touchbar_model::FactValue::Text("light".into()))
    );
    assert!(matches!(
        session
            .control(
                Command::SimFact {
                    name: "surface.kind".into(),
                    value: None
                },
                Some(&fake)
            )
            .0,
        Reply::Error(_)
    ));
}
#[test]
fn control_can_be_polled_between_busy_core_slices_without_waiting_for_idle() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let desktop = Rc::new(touchbar_desktop::NullBackend::default());
        let (effects, _) = touchbar_effects::Engine::fake(desktop.clone());
        let mut session = Session::new(
            SessionOptions::new(dir.path().into(), UserConfig::default()),
            ManualClock::default(),
            Box::<touchbar_logic_runtime::MemoryStore>::default(),
            desktop,
            effects,
        )
        .unwrap();
        for _ in 0..5000 {
            session.handle_event(Event::Wake).unwrap();
        }
        session.pump().unwrap();
        assert!(session.has_pending());
        let mut server = Server::bind(dir.path(), false).unwrap();
        let client = async {
            assert!(matches!(
                call(&socket_path(dir.path(), false), Command::Status)
                    .await
                    .unwrap(),
                Reply::Status(_)
            ));
        };
        let serving = async {
            loop {
                assert!(
                    !touchbar_session::driver::poll_control(&mut server, &mut session, None)
                        .await
                        .unwrap()
                );
                assert!(
                    session.has_pending(),
                    "diagnostics drained unrelated core work"
                );
                async_io::Timer::after(std::time::Duration::from_millis(1)).await;
            }
        };
        futures_lite::future::or(client, serving).await;
    });
}
