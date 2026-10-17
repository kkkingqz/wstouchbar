use touchbar_control::{Command, Theme};
use touchbarctl::ControlOptions;
fn parse(args: &[&str]) -> Result<ControlOptions, String> {
    ControlOptions::parse(args.iter().map(|s| (*s).to_owned()))
}
#[test]
fn control_arguments_select_simulator_explicitly_or_for_sim_commands() {
    assert!(
        parse(&["--simulator", "status", "--json"])
            .unwrap()
            .simulator
    );
    assert!(!parse(&["status"]).unwrap().simulator);
    let options = parse(&["sim", "theme", "light"]).unwrap();
    assert!(options.simulator);
    assert_eq!(
        options.command,
        Command::SimTheme {
            theme: Theme::Light
        }
    );
    let options = parse(&["sim", "fact", "display.brightness=0.4"]).unwrap();
    assert_eq!(
        options.command,
        Command::SimFact {
            name: "display.brightness".into(),
            value: Some(touchbar_model::FactValue::Number(0.4))
        }
    );
    assert_eq!(
        parse(&["sim", "fact", "--unset", "app.id"])
            .unwrap()
            .command,
        Command::SimFact {
            name: "app.id".into(),
            value: None
        }
    );
    assert_eq!(
        parse(&["sim", "fact", "app.id=org.test"]).unwrap().command,
        Command::SimFact {
            name: "app.id".into(),
            value: Some(touchbar_model::FactValue::Text("org.test".into()))
        }
    );
    for args in [
        &["status", "--json", "--json"][..],
        &["sim", "theme", "blue"],
        &["reload", "junk"],
        &["dump", "-o"],
        &["sim", "fact", "foo"],
        &["--simulator", "--simulator", "status"],
    ] {
        assert!(parse(args).is_err(), "{args:?}");
    }
}
#[test]
fn binary_status_and_private_dump_use_real_control_transport() {
    use std::{fs, os::unix::fs::PermissionsExt, process::Command as Process};
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let mut server = touchbar_control::Server::bind(dir.path(), true).unwrap();
        let source = dir.path().join("dump.json");
        fs::write(&source, b"replay data").unwrap();
        let target = dir.path().join("copy.json");
        let core = touchbar_core::Core::new(touchbar_core::CoreConfig::default()).snapshot();
        let status = serde_json::json!({"core":core,"last_calls":[],"load_issues":[],"metrics":{"frame_ms":{"samples":0,"p50":null,"p99":null},"wasm_ms":{"samples":0,"p50":null,"p99":null}}});
        // Blocking child I/O stays on the test's worker thread, not the server reactor.
        let runtime = dir.path().to_owned();
        let output = target.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let child = std::thread::spawn(move || {
            for args in [
                vec!["--simulator", "status", "--json"],
                vec!["status", "--simulator"],
                vec!["--simulator", "dump", "-o", output.to_str().unwrap()],
            ] {
                let result = Process::new(env!("CARGO_BIN_EXE_touchbarctl"))
                    .args(args)
                    .env("XDG_RUNTIME_DIR", &runtime)
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                tx.send(String::from_utf8(result.stdout).unwrap()).unwrap();
            }
        });
        let serving = async {
            loop {
                let incoming = server.next().await.unwrap().expect("control request");
                let reply = match incoming.request.command {
                    Command::Status => touchbar_control::Reply::Status(status.clone()),
                    Command::Dump => touchbar_control::Reply::Dump(source.clone()),
                    _ => panic!("unexpected request"),
                };
                server.respond(incoming, reply).unwrap();
            }
        };
        let clients_done = async {
            loop {
                if child.is_finished() {
                    break;
                }
                async_io::Timer::after(std::time::Duration::from_millis(10)).await;
            }
        };
        futures_lite::future::or(serving, clients_done).await;
        child.join().unwrap();
        let values = rx.iter().collect::<Vec<_>>();
        assert!(
            serde_json::from_str::<serde_json::Value>(&values[0]).unwrap()["core"]["surfaces"]
                .is_array()
        );
        assert!(values[1].contains("0 surface(s)"));
        assert_eq!(fs::read(&target).unwrap(), b"replay data");
        assert_eq!(
            fs::metadata(target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    });
}
