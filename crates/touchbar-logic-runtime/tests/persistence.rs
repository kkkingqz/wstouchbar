use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use touchbar_core::{Input, PersistKey, PersistScope};
use touchbar_logic_runtime::*;
use touchbar_model::*;
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "wstouchbar-persist-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn key(layer: bool) -> PersistKey {
    let set = "owner/main".parse().unwrap();
    PersistKey {
        plugin: "owner".parse().unwrap(),
        surface: "touchbar".parse().unwrap(),
        scope: if layer {
            PersistScope::Layer(set)
        } else {
            PersistScope::ProfileSlot {
                slot: "layers".parse().unwrap(),
                set,
            }
        },
    }
}
fn fields(value: f64) -> StatePatch {
    vec![("value".into(), StateValue::Scalar(Scalar::Number(value)))]
}
#[test]
fn namespaces_are_disjoint_even_for_slot_layers() {
    let t = Temp::new();
    let mut s = FsStore::new(&t.0);
    assert_eq!(
        s.path(&key(false)),
        t.0.join("touchbar/owner/touchbar/slots/layers/main.json")
    );
    assert_eq!(
        s.path(&key(true)),
        t.0.join("touchbar/owner/touchbar/layers/main.json")
    );
    s.write(&key(false), &fields(1.0)).unwrap();
    s.write(&key(true), &fields(2.0)).unwrap();
    assert_eq!(s.load(&key(false)).unwrap(), fields(1.0));
    assert_eq!(s.load(&key(true)).unwrap(), fields(2.0));
}
#[test]
fn partial_writes_merge_and_leave_no_temporary_files() {
    let t = Temp::new();
    let mut s = FsStore::new(&t.0);
    let k = key(true);
    s.write(&k, &fields(1.0)).unwrap();
    s.write(
        &k,
        &vec![(
            "label".into(),
            StateValue::Scalar(Scalar::Text("saved".into())),
        )],
    )
    .unwrap();
    s.write(&k, &fields(2.0)).unwrap();
    let loaded = s.load(&k).unwrap();
    assert_eq!(loaded.len(), 2);
    assert!(loaded.contains(&fields(2.0)[0]));
    assert!(loaded.iter().any(|(k, _)| k == "label"));
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(s.path(&k)).unwrap()).unwrap();
    assert_eq!(json["version"], 1);
    assert!(json["fields"].is_object());
    assert_eq!(
        std::fs::read_dir(s.path(&k).parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}
#[test]
fn corrupt_and_unknown_version_files_are_logged_then_replaced() {
    let t = Temp::new();
    let mut s = FsStore::new(&t.0);
    let k = key(true);
    std::fs::create_dir_all(s.path(&k).parent().unwrap()).unwrap();
    for bytes in [
        b"broken".as_slice(),
        br#"{"version":99,"fields":{}}"#.as_slice(),
    ] {
        s.take_logs();
        std::fs::write(s.path(&k), bytes).unwrap();
        assert!(s.load(&k).unwrap().is_empty());
        assert_eq!(s.take_logs().len(), 1);
        s.write(&k, &fields(3.0)).unwrap();
        assert_eq!(s.load(&k).unwrap(), fields(3.0));
    }
}
#[test]
fn oversized_merged_state_is_rejected_without_replacing_valid_state() {
    let t = Temp::new();
    let mut s = FsStore::new(&t.0);
    let k = key(true);
    s.write(&k, &fields(3.0)).unwrap();
    let before = std::fs::read(s.path(&k)).unwrap();
    assert!(
        s.write(
            &k,
            &vec![(
                "text".into(),
                StateValue::Scalar(Scalar::Text("x".repeat(65_536)))
            )]
        )
        .is_err()
    );
    assert_eq!(std::fs::read(s.path(&k)).unwrap(), before);
    let mut memory = MemoryStore::default();
    memory.write(&k, &fields(3.0)).unwrap();
    assert!(
        memory
            .write(
                &k,
                &vec![(
                    "text".into(),
                    StateValue::Scalar(Scalar::Text("x".repeat(65_536)))
                )]
            )
            .is_err()
    );
    assert_eq!(memory.load(&k).unwrap(), fields(3.0));
}
#[test]
fn ieee_bits_survive_store_round_trip() {
    let t = Temp::new();
    let mut s = FsStore::new(&t.0);
    let original = fields(f64::from_bits(0xfff0000000001234));
    s.write(&key(true), &original).unwrap();
    assert_eq!(
        serde_json::to_value(s.load(&key(true)).unwrap()).unwrap(),
        serde_json::to_value(original).unwrap()
    );
}
fn runtime(store: Box<dyn PersistStore>) -> Runtime {
    Runtime::new(
        RuntimeConfig {
            cache_dir: None,
            limits: touchbar_logic_types::LogicLimits {
                logic_hard_ms: 10_000,
                ..Default::default()
            },
        },
        store,
    )
    .unwrap()
}
fn load(r: &mut Runtime) {
    r.load_module(
        "persist".parse().unwrap(),
        "1",
        &std::fs::read(touchbar_wasm_fixtures::PERSIST).unwrap(),
    )
    .unwrap();
}
fn instantiate(r: &mut Runtime, n: u64) -> LogicUpdate {
    let instance = InstanceId {
        number: n,
        generation: Generation(1),
    };
    let input = r.create(
        Timestamp(0),
        instance,
        &"persist".parse().unwrap(),
        "1",
        &"owner/main".parse().unwrap(),
        &key(true),
    );
    let Input::LogicDescribed { persisted, .. } = input else {
        panic!("description expected")
    };
    match r
        .call(
            Timestamp(0),
            instance,
            CallId(1),
            &LogicCall::Init {
                config: String::new(),
                persisted,
            },
        )
        .input
    {
        Some(Input::LogicUpdate { update, .. }) => update,
        _ => panic!("init update expected"),
    }
}
#[test]
fn new_runtime_and_new_instance_read_persisted_fields() {
    let t = Temp::new();
    let mut r = runtime(Box::new(FsStore::new(&t.0)));
    load(&mut r);
    instantiate(&mut r, 1);
    r.persist(&key(true), &fields(7.0)).unwrap();
    assert!(instantiate(&mut r, 2).state.contains(&fields(7.0)[0]));
    drop(r);
    let mut r = runtime(Box::new(FsStore::new(&t.0)));
    load(&mut r);
    assert!(instantiate(&mut r, 1).state.contains(&fields(7.0)[0]));
}

#[test]
fn harness_emits_persist_after_one_second_and_new_generation_receives_value() {
    use std::{cell::RefCell, rc::Rc};
    use touchbar_core::{CoreConfig, Output};
    use touchbar_testkit::{Harness, LogicReaction, catalog, headless};
    let memory = MemoryStore::default();
    let shared = Rc::new(RefCell::new(runtime(Box::new(memory.clone()))));
    let plugin: PluginId = "owner".parse().unwrap();
    let set: SetRef = "owner/main".parse().unwrap();
    let key = PersistKey {
        plugin: plugin.clone(),
        surface: "headless:1".parse().unwrap(),
        scope: PersistScope::ProfileSlot {
            slot: "layers".parse().unwrap(),
            set: set.clone(),
        },
    };
    shared
        .borrow_mut()
        .load_module(
            plugin.clone(),
            "1",
            &std::fs::read(touchbar_wasm_fixtures::PERSIST).unwrap(),
        )
        .unwrap();
    let probe = InstanceId {
        number: 100,
        generation: Generation(0),
    };
    let Input::LogicDescribed { description, .. } =
        shared
            .borrow_mut()
            .create(Timestamp(0), probe, &plugin, "1", &set, &key)
    else {
        panic!("description expected")
    };
    shared.borrow_mut().dispose(probe);
    let mut cat = catalog()
        .plugin("owner")
        .set("main", |s| {
            s.button("change");
            s.spec.persist = vec!["value".into(), "label".into()];
        })
        .profile("default", |p| {
            p.default().slot("layers", &["owner/main"]);
        })
        .build();
    let mut h = Harness::new(CoreConfig::default());
    h.enable_invariant_checks();
    let backend = shared.clone();
    let storage_key = key.clone();
    // L10 will provide LogicHost. This adapter already runs the real component
    // through the existing Harness; the core still owns timing and cached fields.
    h.logic.register(
        "owner/main",
        Some(description),
        move |instance, invocation| {
            let mut r = backend.borrow_mut();
            if matches!(invocation, LogicCall::Init { .. }) {
                assert!(matches!(
                    r.create(Timestamp(0), instance, &plugin, "1", &set, &storage_key),
                    Input::LogicDescribed { .. }
                ));
            }
            match r.call(Timestamp(0), instance, CallId(1), invocation).input {
                Some(Input::LogicUpdate { update, .. }) => LogicReaction::Update(update),
                Some(Input::LogicError { message, .. }) => LogicReaction::Error(message),
                Some(Input::LogicFault { reason, .. }) => LogicReaction::Fault(reason),
                _ => panic!("logic response expected"),
            }
        },
    );
    h.send(Input::Catalog(cat.clone()));
    h.send(Input::SurfaceAdded(headless(1)));
    h.tap(SurfaceId(1), "change");
    let old = h.snapshot().instances[0].id;
    h.advance_ms(999);
    assert!(
        !h.trace
            .iter()
            .flat_map(|e| &e.outputs)
            .any(|o| matches!(o, Output::Persist { .. }))
    );
    h.advance_ms(1);
    let writes: Vec<_> = h
        .trace
        .iter()
        .flat_map(|e| &e.outputs)
        .filter_map(|o| match o {
            Output::Persist { key, fields, .. } => Some((key.clone(), fields.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(writes.len(), 1);
    for (k, patch) in writes {
        shared.borrow_mut().persist(&k, &patch).unwrap();
    }
    assert!(memory.clone().load(&key).unwrap().contains(&fields(1.0)[0]));
    cat.plugins[0].revision = "next".into();
    let outputs = h.send(Input::Catalog(cat));
    for output in outputs {
        if let Output::DisposeInstance { instance } = output {
            shared.borrow_mut().dispose(instance);
        }
    }
    let snapshot = h.snapshot();
    assert_ne!(snapshot.instances[0].id, old);
    assert!(snapshot.instances[0].state.contains(&fields(1.0)[0]));
    h.finish_invariant_checks().unwrap();
}

#[test]
fn runtime_applies_its_persistence_limit_to_the_merged_state() {
    let memory = MemoryStore::with_limit(100_000);
    let mut r = Runtime::new(
        RuntimeConfig {
            cache_dir: None,
            limits: touchbar_logic_types::LogicLimits {
                max_persist_bytes: 128,
                ..Default::default()
            },
        },
        Box::new(memory.clone()),
    )
    .unwrap();
    r.persist(&key(true), &fields(1.0)).unwrap();
    assert!(
        r.persist(
            &key(true),
            &vec![(
                "large".into(),
                StateValue::Scalar(Scalar::Text("x".repeat(500)))
            )]
        )
        .is_err()
    );
    assert_eq!(memory.clone().load(&key(true)).unwrap(), fields(1.0));
}
#[test]
fn oversized_file_is_logged_and_replaced_by_next_valid_write() {
    let t = Temp::new();
    let mut s = FsStore::new(&t.0);
    let path = s.path(&key(true));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, vec![b'x'; 65_537]).unwrap();
    assert!(s.load(&key(true)).unwrap().is_empty());
    assert_eq!(s.take_logs().len(), 1);
    s.write(&key(true), &fields(1.0)).unwrap();
    assert_eq!(s.load(&key(true)).unwrap(), fields(1.0));
}
