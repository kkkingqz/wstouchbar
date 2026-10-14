use touchbar_core::{Input, PersistKey, PersistScope};
use touchbar_logic_runtime::*;
use touchbar_model::*;
use touchbar_wasm_fixtures as fixtures;
fn plugin() -> PluginId {
    "echo".parse().unwrap()
}
fn id(n: u64) -> InstanceId {
    InstanceId {
        number: n,
        generation: Generation(1),
    }
}
fn set() -> SetRef {
    "owner/echo".parse().unwrap()
}
fn key() -> PersistKey {
    PersistKey {
        plugin: "owner".parse().unwrap(),
        surface: "touchbar".parse().unwrap(),
        scope: PersistScope::Layer(set()),
    }
}
fn runtime() -> Runtime {
    Runtime::new(
        RuntimeConfig {
            cache_dir: None,
            limits: touchbar_logic_types::LogicLimits {
                logic_hard_ms: 10_000,
                ..Default::default()
            },
        },
        Box::<MemoryStore>::default(),
    )
    .unwrap()
}
fn load(r: &mut Runtime, revision: &str, path: &str) {
    r.load_module(plugin(), revision, &std::fs::read(path).unwrap())
        .unwrap();
}
fn create(r: &mut Runtime, n: u64, revision: &str) {
    assert!(matches!(
        r.create(Timestamp(0), id(n), &plugin(), revision, &set(), &key()),
        Input::LogicDescribed { .. }
    ));
}
fn update(r: &mut Runtime, n: u64, call: LogicCall) -> LogicUpdate {
    match r.call(Timestamp(42), id(n), CallId(7), &call).input {
        Some(Input::LogicUpdate {
            instance,
            call,
            update,
        }) => {
            assert_eq!(instance, id(n));
            assert_eq!(call, CallId(7));
            update
        }
        other => panic!("unexpected {other:?}"),
    }
}
fn init(r: &mut Runtime, n: u64) {
    update(
        r,
        n,
        LogicCall::Init {
            config: String::new(),
            persisted: vec![],
        },
    );
}
#[test]
fn explicit_revision_preserves_live_instances_and_old_catalog() {
    let mut r = runtime();
    load(&mut r, "old", fixtures::ECHO);
    create(&mut r, 1, "old");
    init(&mut r, 1);
    load(&mut r, "new", fixtures::ERR);
    // A failed catalog reload must still be able to create old logic.
    create(&mut r, 2, "old");
    init(&mut r, 2);
    // Cached revision is immutable; even invalid bytes do not replace it.
    r.load_module(plugin(), "old", b"invalid").unwrap();
    assert_eq!(
        update(&mut r, 1, LogicCall::Handle(LogicEvent::Shown)),
        update(&mut r, 2, LogicCall::Handle(LogicEvent::Shown))
    );
    create(&mut r, 3, "new");
    init(&mut r, 3);
    assert!(matches!(
        r.call(
            Timestamp(0),
            id(3),
            CallId(9),
            &LogicCall::Handle(LogicEvent::Shown)
        )
        .input,
        Some(Input::LogicError {
            call: CallId(9),
            ..
        })
    ));
    r.dispose(id(1));
    r.dispose(id(1));
    let ignored = r.call(
        Timestamp(0),
        id(1),
        CallId(9),
        &LogicCall::Handle(LogicEvent::Shown),
    );
    assert!(ignored.input.is_none());
    assert!(!ignored.logs.is_empty());
    update(&mut r, 2, LogicCall::Handle(LogicEvent::Hidden));
}
#[test]
fn unsupported_version_and_wasi_are_precise_instance_faults() {
    for (path, needle) in [
        (fixtures::VERSION, "wstouchbar:logic/api@9.0.0"),
        (fixtures::WASI, "wasi:cli/environment"),
    ] {
        let mut r = runtime();
        let error = r
            .load_module(plugin(), "bad", &std::fs::read(path).unwrap())
            .unwrap_err();
        assert!(error.to_string().contains(needle), "{error}");
        match r.create(Timestamp(0), id(1), &plugin(), "bad", &set(), &key()) {
            Input::InstanceFault { instance, reason } => {
                assert_eq!(instance, id(1));
                assert!(reason.contains(needle), "{reason}")
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
#[test]
fn identical_sequences_on_two_runtimes_are_deterministic() {
    let mut a = runtime();
    let mut b = runtime();
    load(&mut a, "1", fixtures::ECHO);
    load(&mut b, "1", fixtures::ECHO);
    create(&mut a, 1, "1");
    create(&mut b, 1, "1");
    init(&mut a, 1);
    init(&mut b, 1);
    for event in [LogicEvent::Shown, LogicEvent::Hidden, LogicEvent::Shown] {
        assert_eq!(
            update(&mut a, 1, LogicCall::Handle(event.clone())),
            update(&mut b, 1, LogicCall::Handle(event))
        );
    }
}
#[path = "support/nan.rs"]
mod nan_support;
use nan_support::{normalize_event, normalize_update};
use proptest::prelude::*;
use touchbar_testkit::logic_strategies::*;
std::thread_local! {static ECHO:std::cell::RefCell<Runtime> =std::cell::RefCell::new({let mut r=runtime();load(&mut r,"1",fixtures::ECHO);r});}
proptest! {
 #[test] fn echo_updates_cross_component_without_payload_loss(expected in logic_update_strategy()) {
  ECHO.with(|r|{let mut r=r.borrow_mut();r.dispose(id(1));create(&mut r,1,"1");let received=update(&mut r,1,LogicCall::Init{config:serde_json::to_string(&expected).unwrap(),persisted:vec![]});prop_assert_eq!(normalize_update(received),normalize_update(expected));Ok(())})?;
 }
 #[test] fn echo_events_cross_component_without_payload_loss(expected in logic_event_strategy()) {
  ECHO.with(|r|{let mut r=r.borrow_mut();r.dispose(id(1));create(&mut r,1,"1");init(&mut r,1);let received=update(&mut r,1,LogicCall::Handle(expected.clone()));let StateValue::Scalar(Scalar::Text(json))=&received.state[0].1 else{panic!("missing echo")};let decoded:serde_json::Value=serde_json::from_str(json).unwrap();assert_eq!(decoded["context"]["now_ms"],42);let event=serde_json::from_value(decoded["event"].clone()).unwrap();prop_assert_eq!(normalize_event(event),normalize_event(expected));Ok(())})?;
 }
}
#[test]
fn nan_normalization_does_not_change_text_tags() {
    let u = LogicUpdate {
        state: vec![(
            "text".into(),
            StateValue::Scalar(Scalar::Text("f64:7ff0000000000001".into())),
        )],
        ..Default::default()
    };
    assert_eq!(
        normalize_update(u.clone()),
        serde_json::to_value(u).unwrap()
    );
}

#[test]
fn description_is_cached_once_per_revision() {
    let mut r = runtime();
    load(&mut r, "1", fixtures::ECHO);
    create(&mut r, 1, "1");
    assert_eq!(
        r.take_logs()
            .iter()
            .filter(|l| l.message == "echo.describe")
            .count(),
        1
    );
    create(&mut r, 2, "1");
    assert!(r.take_logs().is_empty());
    load(&mut r, "2", fixtures::ECHO);
    create(&mut r, 3, "2");
    assert_eq!(
        r.take_logs()
            .iter()
            .filter(|l| l.message == "echo.describe")
            .count(),
        1
    );
}

#[test]
fn cache_configuration_is_applied_and_invalid_paths_are_rejected() {
    let bad = Runtime::new(
        RuntimeConfig {
            cache_dir: Some("relative/cache".into()),
            ..Default::default()
        },
        Box::<MemoryStore>::default(),
    );
    assert!(bad.is_err(), "relative cache path must be rejected");
    let root = std::env::temp_dir().join(format!("wstouchbar-l6-cache-{}", std::process::id()));
    let mut r = Runtime::new(
        RuntimeConfig {
            cache_dir: Some(root.clone()),
            ..Default::default()
        },
        Box::<MemoryStore>::default(),
    )
    .unwrap();
    load(&mut r, "1", fixtures::ECHO);
    assert!(root.is_dir());
    assert!(
        std::fs::read_dir(&root).unwrap().next().is_some(),
        "compiled component must populate cache"
    );
    drop(r);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn worker_prepares_module_while_runtime_keeps_live_instance() {
    let mut r = runtime();
    load(&mut r, "old", fixtures::ECHO);
    create(&mut r, 1, "old");
    init(&mut r, 1);
    let compiler = r.compiler();
    let worker =
        std::thread::spawn(move || compiler.prepare(&std::fs::read(fixtures::ECHO).unwrap()));
    update(&mut r, 1, LogicCall::Handle(LogicEvent::Shown));
    let prepared = worker.join().unwrap();
    assert!(prepared.is_ok());
    r.install(plugin(), "worker", prepared).unwrap();
    create(&mut r, 2, "worker");
    init(&mut r, 2);
    assert_eq!(
        update(&mut r, 1, LogicCall::Handle(LogicEvent::Shown)),
        update(&mut r, 2, LogicCall::Handle(LogicEvent::Shown))
    );
    // Both success and failure records remain immutable on installation.
    r.install(plugin(), "worker", Err(LoadError("replacement".into())))
        .unwrap();
    r.install(
        plugin(),
        "failed",
        Err(LoadError("failed preparation".into())),
    )
    .unwrap_err();
    let prepared = r
        .compiler()
        .prepare(&std::fs::read(fixtures::ECHO).unwrap());
    assert!(
        r.install(plugin(), "failed", prepared)
            .unwrap_err()
            .0
            .contains("failed preparation")
    );
}
#[test]
fn prepared_module_from_another_engine_is_rejected_without_affecting_live_guest() {
    let mut r = runtime();
    load(&mut r, "old", fixtures::ECHO);
    create(&mut r, 1, "old");
    init(&mut r, 1);
    let other = runtime();
    let prepared = other
        .compiler()
        .prepare(&std::fs::read(fixtures::ECHO).unwrap());
    let error = r.install(plugin(), "foreign", prepared).unwrap_err();
    assert!(error.0.contains("engine"), "{error}");
    update(&mut r, 1, LogicCall::Handle(LogicEvent::Shown));
}

#[test]
fn module_retention_evicts_old_records_without_disposing_live_instances() {
    let mut r = runtime();
    load(&mut r, "old", fixtures::ECHO);
    create(&mut r, 1, "old");
    init(&mut r, 1);
    load(&mut r, "accepted", fixtures::ECHO);
    r.load_module(plugin(), "failed", b"invalid").unwrap_err();
    r.retain_modules(|p, revision| p == &plugin() && revision == "accepted");
    for revision in ["old", "failed"] {
        assert!(
            matches!(r.create(Timestamp(0), id(3), &plugin(), revision, &set(), &key()), Input::InstanceFault {ref reason, ..} if reason.contains("not loaded"))
        );
    }
    update(&mut r, 1, LogicCall::Handle(LogicEvent::Shown));
    create(&mut r, 2, "accepted");
    init(&mut r, 2);
    // A failed record really was removed: valid preparation under that key can succeed.
    load(&mut r, "failed", fixtures::ECHO);
    create(&mut r, 3, "failed");
    init(&mut r, 3);
}

#[test]
fn loader_describe_shares_create_cache_and_keeps_guest_state_fresh() {
    let mut r = runtime();
    load(&mut r, "loader", fixtures::ECHO);
    let description = r.describe(&plugin(), "loader").unwrap();
    assert_eq!(
        r.take_logs()
            .iter()
            .filter(|l| l.message == "echo.describe")
            .count(),
        1
    );
    assert_eq!(r.describe(&plugin(), "loader").unwrap(), description);
    match r.create(Timestamp(0), id(90), &plugin(), "loader", &set(), &key()) {
        Input::LogicDescribed {
            description: actual,
            ..
        } => assert_eq!(actual, description),
        other => panic!("{other:?}"),
    }
    assert!(r.take_logs().is_empty());
    init(&mut r, 90);
    assert!(r.describe(&plugin(), "missing").is_err());
}
