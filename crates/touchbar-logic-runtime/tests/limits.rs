use touchbar_core::{Input, PersistKey, PersistScope};
use touchbar_logic_runtime::*;
use touchbar_logic_types::LogicLimits;
use touchbar_model::*;
use touchbar_wasm_fixtures as f;
#[path = "support/tables.rs"]
mod tables;

#[test]
fn real_component_table_allocations_are_bounded_and_neighbor_survives() {
    table_case(700, false);
}
#[test]
fn real_component_table_growth_is_bounded_and_neighbor_survives() {
    table_case(0, true);
}
fn table_case(initial: u64, grow: bool) {
    let mut r = runtime(LogicLimits {
        max_table_elements: 1024,
        ..limits()
    });
    let plugin: PluginId = "tables".parse().unwrap();
    r.load_module(plugin.clone(), "1", &tables::component(initial, grow))
        .unwrap();
    create(&mut r, 2, "good", f::ECHO);
    success(init(&mut r, 2, ""));
    let set = "owner/test".parse().unwrap();
    let key = PersistKey {
        plugin: "owner".parse().unwrap(),
        surface: "touchbar".parse().unwrap(),
        scope: PersistScope::Layer(set),
    };
    let described = r.create(
        Timestamp(0),
        id(1),
        &plugin,
        "1",
        &"owner/test".parse().unwrap(),
        &key,
    );
    if grow {
        assert!(
            matches!(described, Input::LogicDescribed { .. }),
            "{described:?}"
        );
        success(init(&mut r, 1, ""));
        fault(
            call(&mut r, 1, LogicCall::Handle(LogicEvent::Shown)),
            "table",
        );
    } else {
        assert!(
            matches!(described, Input::InstanceFault { ref reason, .. } if reason.starts_with("table")),
            "{described:?}"
        );
    }
    assert!(
        call(&mut r, 1, LogicCall::Handle(LogicEvent::Shown))
            .input
            .is_none()
    );
    success(call(&mut r, 2, LogicCall::Handle(LogicEvent::Shown)));
    quiet_fault_dispose(&mut r, id(1));
}
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
        quiet_fault_dispose(&mut r, id(1));
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
fn oversized_updates_fault_but_host_inputs_preserve_instances() {
    let mut r = runtime(LogicLimits {
        max_update_bytes: 4096,
        ..limits()
    });
    create(&mut r, 1, "budget", f::BUDGET);
    create(&mut r, 2, "echo", f::ECHO);
    fault(init(&mut r, 1, ""), "budget");
    quiet_fault_dispose(&mut r, id(1));
    success(init(&mut r, 2, ""));
    let rejected = init(&mut r, 2, &"x".repeat(5000));
    assert!(
        matches!(rejected.input, Some(Input::LogicError { ref message, .. }) if message.contains("host input budget"))
    );
    assert!(rejected.logs.iter().any(|l| l.level == LogLevel::Error));
    success(call(&mut r, 2, LogicCall::Handle(LogicEvent::Shown)));
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

#[test]
fn fault_dispose_acknowledges_only_failed_generation_once() {
    let mut r = runtime(limits());
    create(&mut r, 1, "bad", f::TRAP);
    success(init(&mut r, 1, "trap"));
    fault(call(&mut r, 1, action("trap")), "trap");
    r.take_logs();
    let next = InstanceId {
        generation: Generation(2),
        ..id(1)
    };
    r.dispose(next);
    assert!(
        r.take_logs()
            .iter()
            .any(|l| l.message.contains("unknown instance"))
    );
    let set: SetRef = "owner/test".parse().unwrap();
    let key = PersistKey {
        plugin: "owner".parse().unwrap(),
        surface: "touchbar".parse().unwrap(),
        scope: PersistScope::Layer(set.clone()),
    };
    assert!(matches!(
        r.create(Timestamp(0), next, &"bad".parse().unwrap(), "1", &set, &key),
        Input::LogicDescribed { .. }
    ));
    success(r.call(
        Timestamp(0),
        next,
        CallId(20),
        &LogicCall::Init {
            config: "trap".into(),
            persisted: vec![],
        },
    ));
    quiet_fault_dispose(&mut r, id(1));
    success(r.call(
        Timestamp(0),
        next,
        CallId(21),
        &LogicCall::Handle(LogicEvent::Shown),
    ));
    r.dispose(id(1));
    assert!(
        r.take_logs()
            .iter()
            .any(|l| l.message.contains("unknown instance"))
    );
}
fn quiet_fault_dispose(r: &mut Runtime, failed: InstanceId) {
    r.take_logs();
    r.dispose(failed);
    assert!(
        !r.take_logs()
            .iter()
            .any(|l| l.message.contains("unknown instance"))
    );
}
#[test]
fn failed_create_dispose_is_quiet_but_duplicate_create_does_not_leave_ack() {
    let mut r = runtime(limits());
    let plugin: PluginId = "bad".parse().unwrap();
    let set: SetRef = "owner/test".parse().unwrap();
    let key = PersistKey {
        plugin: "owner".parse().unwrap(),
        surface: "touchbar".parse().unwrap(),
        scope: PersistScope::Layer(set.clone()),
    };
    for revision in ["missing", "invalid"] {
        if revision == "invalid" {
            r.load_module(plugin.clone(), revision, b"invalid")
                .unwrap_err();
        }
        assert!(matches!(
            r.create(Timestamp(0), id(1), &plugin, revision, &set, &key),
            Input::InstanceFault { .. }
        ));
        quiet_fault_dispose(&mut r, id(1));
    }
    create(&mut r, 1, "good", f::ECHO);
    assert!(matches!(
        r.create(
            Timestamp(0),
            id(1),
            &"good".parse().unwrap(),
            "1",
            &set,
            &key
        ),
        Input::InstanceFault { .. }
    ));
    success(init(&mut r, 1, ""));
    r.dispose(id(1));
    r.take_logs();
    r.dispose(id(1));
    assert!(
        r.take_logs()
            .iter()
            .any(|l| l.message.contains("unknown instance"))
    );
}
