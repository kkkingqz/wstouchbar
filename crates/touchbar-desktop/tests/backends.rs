use touchbar_desktop::*;
use touchbar_model::*;
fn drain(
    rx: &async_channel::Receiver<BackendEvent>,
    monitor: &mut BackendMonitor,
    now: u64,
) -> Vec<FactChange> {
    let mut out = vec![];
    while let Ok(event) = rx.try_recv() {
        out.extend(monitor.apply(event, Timestamp(now)).unwrap().facts);
    }
    out
}
#[test]
fn null_advertises_nothing_and_every_command_is_unsupported() {
    let (tx, rx) = async_channel::bounded(32);
    let mut backend = NullBackend::default();
    assert_eq!(backend.name(), "null");
    backend.start(tx).unwrap();
    assert_eq!(
        rx.try_recv().unwrap(),
        BackendEvent::Ready(Capabilities::default())
    );
    for command in commands() {
        assert_eq!(
            futures_lite::future::block_on(backend.command(command)),
            Err(DesktopError::Unsupported)
        );
    }
    assert!(rx.is_empty());
}
fn commands() -> Vec<DesktopCommand> {
    vec![
        DesktopCommand::Shortcut {
            keys: vec!["CTRL".into(), "T".into()],
            app: Some("org.example.App".into()),
        },
        DesktopCommand::Key {
            key: SystemKey::F1,
            phase: KeyPhase::Pressed,
        },
        DesktopCommand::BrightnessSet { level: 0.4 },
        DesktopCommand::WorkspaceActivate { id: "2".into() },
        DesktopCommand::AppLaunch {
            id: "org.example.App".into(),
        },
    ]
}
#[test]
fn fake_facts_and_commands_are_in_memory_and_emit_brightness() {
    let (tx, rx) = async_channel::bounded(32);
    let mut backend = FakeBackend::default();
    backend.start(tx).unwrap();
    let mut monitor = BackendMonitor::default();
    drain(&rx, &mut monitor, 0);
    backend
        .set_fact(
            "app.id".into(),
            Some(FactValue::Text("org.example.App".into())),
        )
        .unwrap();
    backend
        .set_fact("custom.records".into(), Some(FactValue::Records(vec![])))
        .unwrap();
    drain(&rx, &mut monitor, 0);
    assert_eq!(
        monitor.facts()[&"app.id".into()],
        FactValue::Text("org.example.App".into())
    );
    for command in commands() {
        futures_lite::future::block_on(backend.command(command)).unwrap();
    }
    let changes = drain(&rx, &mut monitor, 0);
    assert!(changes.contains(&("display.brightness".into(), Some(FactValue::Number(0.4)))));
    assert_eq!(backend.log(), commands());
    assert_eq!(
        backend.fact(&"display.brightness".into()),
        Some(FactValue::Number(0.4))
    );
    backend.set_fact("app.id".into(), None).unwrap();
    drain(&rx, &mut monitor, 0);
    assert!(!monitor.facts().contains_key(&"app.id".into()));
}
#[test]
fn loss_removes_all_owned_facts_and_capabilities_and_reconnect_restores_snapshot() {
    let (tx, rx) = async_channel::bounded(32);
    let mut backend = FakeBackend::default();
    backend.start(tx).unwrap();
    let mut monitor = BackendMonitor::default();
    drain(&rx, &mut monitor, 0);
    backend
        .set_fact("app.id".into(), Some(FactValue::Text("example".into())))
        .unwrap();
    drain(&rx, &mut monitor, 0);
    let owned = monitor.facts().len();
    backend.lose("test disconnect").unwrap();
    let cleared = drain(&rx, &mut monitor, 20);
    assert_eq!(cleared.len(), owned);
    assert!(cleared.iter().all(|(_, v)| v.is_none()));
    assert!(monitor.facts().is_empty());
    assert_eq!(monitor.capabilities(), &Capabilities::default());
    assert_eq!(monitor.retry_at(), Some(Timestamp(1020)));
    assert!(matches!(
        futures_lite::future::block_on(
            backend.command(DesktopCommand::BrightnessSet { level: 0.9 })
        ),
        Err(DesktopError::Disconnected)
    ));
    assert!(!monitor.begin_reconnect(Timestamp(1019)));
    assert!(monitor.begin_reconnect(Timestamp(1020)));
    backend.reconnect().unwrap();
    drain(&rx, &mut monitor, 1020);
    assert!(matches!(monitor.status(), BackendStatus::Ready));
    assert_eq!(
        monitor.facts()[&"app.id".into()],
        FactValue::Text("example".into())
    );
    backend.lose("again").unwrap();
    drain(&rx, &mut monitor, 2000);
    assert_eq!(monitor.retry_at(), Some(Timestamp(3000)));
}
#[test]
fn failed_connections_back_off_and_stale_facts_are_ignored() {
    let mut monitor = BackendMonitor::default();
    let mut now = 0u64;
    let mut expected = 1000;
    for _ in 0..10 {
        monitor
            .apply(BackendEvent::Lost("absent".into()), Timestamp(now))
            .unwrap();
        assert_eq!(monitor.retry_at(), Some(Timestamp(now + expected)));
        assert!(
            monitor
                .apply(
                    BackendEvent::Fact("app.id".into(), Some(FactValue::Text("stale".into()))),
                    Timestamp(now)
                )
                .unwrap()
                .facts
                .is_empty()
        );
        now += expected;
        assert!(monitor.begin_reconnect(Timestamp(now)));
        expected = (expected * 2).min(60_000);
    }
}
#[test]
fn fake_rejects_invalid_input_and_full_channel_without_mutating_state() {
    let (tx, rx) = async_channel::bounded(4);
    let mut backend = FakeBackend::default();
    backend.start(tx).unwrap();
    while rx.try_recv().is_ok() {}
    assert!(
        backend
            .set_fact("".into(), Some(FactValue::Boolean(true)))
            .is_err()
    );
    assert!(
        backend
            .set_fact("surface.width".into(), Some(FactValue::Number(10.0)))
            .is_err()
    );
    assert!(
        backend
            .set_fact("number".into(), Some(FactValue::Number(f64::NAN)))
            .is_err()
    );
    assert!(
        futures_lite::future::block_on(
            backend.command(DesktopCommand::BrightnessSet { level: 1.1 })
        )
        .is_err()
    );
    assert!(backend.log().is_empty());
    for i in 0..4 {
        backend
            .set_fact(format!("f{i}").into(), Some(FactValue::Boolean(true)))
            .unwrap();
    }
    assert!(
        backend
            .set_fact("overflow".into(), Some(FactValue::Boolean(true)))
            .is_err()
    );
    assert_eq!(backend.fact(&"overflow".into()), None);
}
#[test]
fn backend_wire_types_roundtrip() {
    let event = BackendEvent::Ready(Capabilities {
        facts: ["app.id".into()].into(),
        commands: [CommandKind::Shortcut].into(),
    });
    assert_eq!(
        serde_json::from_str::<BackendEvent>(&serde_json::to_string(&event).unwrap()).unwrap(),
        event
    );
    for command in commands() {
        assert_eq!(
            serde_json::from_str::<DesktopCommand>(&serde_json::to_string(&command).unwrap())
                .unwrap(),
            command
        );
    }
}
