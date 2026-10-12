use touchbar_core::{Input, PersistKey, PersistScope};
use touchbar_logic_runtime::*;
use touchbar_logic_types::LogicLimits;
use touchbar_model::*;
use touchbar_wasm_fixtures as f;
fn id(n: u64) -> InstanceId {
    InstanceId {
        number: n,
        generation: Generation(1),
    }
}
fn create(r: &mut Runtime, n: u64, plugin: &str, path: &str) {
    let plugin: PluginId = plugin.parse().unwrap();
    r.load_module(plugin.clone(), "1", &std::fs::read(path).unwrap())
        .unwrap();
    let set: SetRef = "owner/test".parse().unwrap();
    let key = PersistKey {
        plugin: "owner".parse().unwrap(),
        surface: "touchbar".parse().unwrap(),
        scope: PersistScope::Layer(set.clone()),
    };
    assert!(matches!(
        r.create(Timestamp(0), id(n), &plugin, "1", &set, &key),
        Input::LogicDescribed { .. }
    ));
}
fn runtime(limits: LogicLimits) -> Runtime {
    Runtime::new(
        RuntimeConfig {
            limits,
            cache_dir: None,
        },
        Box::<MemoryStore>::default(),
    )
    .unwrap()
}
fn limits() -> LogicLimits {
    LogicLimits {
        logic_hard_ms: 10_000,
        ..Default::default()
    }
}
fn call(r: &mut Runtime, n: u64, c: LogicCall) -> CallResponse {
    r.call(Timestamp(1), id(n), CallId(17), &c)
}
fn init(r: &mut Runtime, n: u64, config: &str) -> CallResponse {
    call(
        r,
        n,
        LogicCall::Init {
            config: config.into(),
            persisted: vec![],
        },
    )
}
fn action(name: &str) -> LogicCall {
    LogicCall::Handle(LogicEvent::Action(ActionEvent {
        action: name.into(),
        kind: ActionKind::Activated,
        value: None,
        record_key: None,
        origin: Origin::Physical,
        cancel_reason: None,
    }))
}
fn fault(response: CallResponse, needle: &str) {
    match response.input {
        Some(Input::LogicFault {
            call: CallId(17),
            reason,
            ..
        }) => assert!(reason.starts_with(needle), "{reason}"),
        _ => panic!("expected LogicFault"),
    }
}
fn success(response: CallResponse) {
    assert!(
        matches!(response.input, Some(Input::LogicUpdate { .. })),
        "{:?}",
        response.input
    )
}
#[test]
fn real_traps_timeouts_memory_and_stack_faults_are_isolated() {
    for (path, action_name, reason) in [
        (f::TRAP, "trap", "trap"),
        (f::HANG, "hang", "timeout"),
        (f::OOM, "oom", "memory"),
        (f::TRAP, "stack", "stack"),
    ] {
        let mut r = runtime(LogicLimits {
            logic_hard_ms: if action_name == "hang" { 25 } else { 10_000 },
            max_memory_bytes: 4 * 1024 * 1024,
            ..limits()
        });
        create(&mut r, 1, "bad", path);
        create(&mut r, 2, "bad", path);
        create(&mut r, 3, "good", f::ECHO);
        success(init(&mut r, 1, action_name));
        success(init(&mut r, 2, action_name));
        success(init(&mut r, 3, ""));
        fault(call(&mut r, 1, action(action_name)), reason);
        assert!(
            call(&mut r, 1, LogicCall::Handle(LogicEvent::Shown))
                .input
                .is_none()
        );
        success(call(&mut r, 2, LogicCall::Handle(LogicEvent::Shown)));
        success(call(&mut r, 3, LogicCall::Handle(LogicEvent::Shown)));
    }
}
#[test]
fn oversized_updates_and_calls_fault_without_affecting_neighbor() {
    let mut r = runtime(LogicLimits {
        max_update_bytes: 4096,
        ..limits()
    });
    create(&mut r, 1, "budget", f::BUDGET);
    create(&mut r, 2, "echo", f::ECHO);
    fault(init(&mut r, 1, ""), "budget");
    success(init(&mut r, 2, ""));
    fault(init(&mut r, 2, &"x".repeat(5000)), "budget");
    assert!(
        call(&mut r, 2, LogicCall::Handle(LogicEvent::Shown))
            .input
            .is_none()
    );
    create(&mut r, 3, "echo", f::ECHO);
    success(init(&mut r, 3, ""));
    success(call(&mut r, 3, LogicCall::Handle(LogicEvent::Shown)));
}
#[test]
fn guest_errors_keep_runtime_instance_usable() {
    let mut r = runtime(limits());
    create(&mut r, 1, "err", f::ERR);
    assert!(matches!(
        init(&mut r, 1, "init").input,
        Some(Input::LogicError {
            call: CallId(17),
            ..
        })
    ));
    success(init(&mut r, 1, "handle"));
    for _ in 0..2 {
        assert!(matches!(
            call(&mut r, 1, LogicCall::Handle(LogicEvent::Shown)).input,
            Some(Input::LogicError { .. })
        ));
    }
    success(init(&mut r, 1, "ok"));
    success(call(&mut r, 1, LogicCall::Handle(LogicEvent::Shown)));
}
#[test]
fn guest_logging_is_bounded_utf8_safe_and_resets_each_call() {
    let mut r = runtime(LogicLimits {
        max_log_records: 3,
        max_log_message_bytes: 8,
        ..limits()
    });
    create(&mut r, 1, "log", f::LOG);
    for response in [
        init(&mut r, 1, "30"),
        call(&mut r, 1, LogicCall::Handle(LogicEvent::Shown)),
    ] {
        success(CallResponse {
            input: response.input,
            logs: vec![],
            duration: response.duration,
        });
        let info: Vec<_> = response
            .logs
            .iter()
            .filter(|l| l.level == LogLevel::Info)
            .collect();
        assert_eq!(info.len(), 3);
        assert!(info.iter().all(|l| l.message.len() <= 8));
        assert_eq!(info[0].message, "0: жж");
        let counters: Vec<_> = response
            .logs
            .iter()
            .filter(|l| l.message.contains("997") && l.message.contains("dropped"))
            .collect();
        assert_eq!(counters.len(), 1);
    }
}
#[test]
fn soft_deadline_warns_without_interrupting_call() {
    let mut r = runtime(LogicLimits {
        logic_soft_ms: 0,
        ..limits()
    });
    create(&mut r, 1, "echo", f::ECHO);
    let response = init(&mut r, 1, "");
    assert!(response.duration > std::time::Duration::ZERO);
    assert!(
        response
            .logs
            .iter()
            .any(|l| l.level == LogLevel::Warn && l.message.contains("soft"))
    );
    success(response);
}
