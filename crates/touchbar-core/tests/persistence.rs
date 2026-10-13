use touchbar_core::*;
use touchbar_model::*;

fn catalog() -> Catalog {
    let mut set = SetSpec::default_for("p/main".parse().unwrap());
    set.logic = Some("shared".parse().unwrap());
    let mut p = ProfileSpec::default_for("p/default".parse().unwrap());
    p.default = true;
    p.slots.push(SlotSpec {
        id: "main".parse().unwrap(),
        candidates: vec![set.reference.clone()],
    });
    Catalog {
        plugins: vec!["p", "shared"]
            .into_iter()
            .map(|name| PluginSpec {
                id: name.parse().unwrap(),
                revision: "1".into(),
                enabled: true,
                has_logic: true,
                requires: vec![],
            })
            .collect(),
        sets: vec![set],
        profiles: vec![p],
        bindings: vec![Binding {
            id: "held".into(),
            when: None,
            trigger: Trigger::Key(KeyTrigger {
                keys: vec![WatchKey::Fn],
                gesture: KeyGesture::WhileHeld,
            }),
            action: LayerAction {
                operation: LayerOperation::Layer("p/main".parse().unwrap()),
                surfaces: LayerSurfaces::Source,
            },
        }],
        ..Catalog::default()
    }
}
fn info(id: u64) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(id),
        name: "headless:stable".parse().unwrap(),
        kind: SurfaceKind::Headless,
        width_px: 500,
        height_px: 50,
        px_per_mm: 5.0,
        caps: SurfaceCaps {
            watch_keys: true,
            ..SurfaceCaps::default()
        },
    }
}
fn setup(config: CoreConfig) -> (Core, InstanceId, PersistKey) {
    let mut c = Core::new(config);
    c.handle(Timestamp(0), Input::Catalog(catalog()));
    let out = c.handle(Timestamp(0), Input::SurfaceAdded(info(1)));
    let (id, key) = out
        .iter()
        .find_map(|o| {
            if let Output::CreateInstance {
                instance,
                persist_key,
                ..
            } = o
            {
                Some((*instance, persist_key.clone()))
            } else {
                None
            }
        })
        .unwrap();
    (c, id, key)
}
fn fields() -> LogicDescription {
    LogicDescription {
        state: vec![
            StateField {
                name: "level".into(),
                kind: FieldKind::Scalar(ScalarKind::Number),
                persist: true,
            },
            StateField {
                name: "text".into(),
                kind: FieldKind::Scalar(ScalarKind::Text),
                persist: true,
            },
            StateField {
                name: "volatile".into(),
                kind: FieldKind::Scalar(ScalarKind::Number),
                persist: false,
            },
        ],
        ..LogicDescription::default()
    }
}
fn number(v: f64) -> StateValue {
    StateValue::Scalar(Scalar::Number(v))
}
fn describe(c: &mut Core, id: InstanceId, at: u64, persisted: StatePatch) -> Vec<Output> {
    c.handle(
        Timestamp(at),
        Input::LogicDescribed {
            instance: id,
            description: fields(),
            persisted,
        },
    )
}
fn reply(c: &mut Core, id: InstanceId, at: u64, state: StatePatch) -> Vec<Output> {
    let call = c
        .snapshot()
        .instances
        .iter()
        .find(|i| i.id == id)
        .unwrap()
        .in_flight
        .unwrap();
    c.handle(
        Timestamp(at),
        Input::LogicUpdate {
            instance: id,
            call,
            update: LogicUpdate {
                state,
                ..LogicUpdate::default()
            },
        },
    )
}
fn timer(out: &[Output]) -> (TimerId, Timestamp) {
    out.iter()
        .find_map(|o| {
            if let Output::SetTimer { id, at } = o {
                Some((*id, *at))
            } else {
                None
            }
        })
        .expect("persistence must schedule a timer")
}
fn saves(out: &[Output]) -> Vec<(PersistKey, StatePatch)> {
    out.iter()
        .filter_map(|o| {
            if let Output::Persist { key, fields, .. } = o {
                Some((key.clone(), fields.clone()))
            } else {
                None
            }
        })
        .collect()
}
#[test]
fn disk_fields_are_schema_filtered_and_bounded_before_init() {
    let (mut c, id, _) = setup(CoreConfig {
        max_text_bytes: 3,
        ..CoreConfig::default()
    });
    let out = describe(
        &mut c,
        id,
        1,
        vec![
            ("level".into(), number(3.0)),
            ("volatile".into(), number(2.0)),
            ("unknown".into(), number(0.0)),
            (
                "text".into(),
                StateValue::Scalar(Scalar::Text("too long".into())),
            ),
        ],
    );
    assert!(out.iter().any(|o|matches!(o,Output::CallLogic {invocation:LogicCall::Init {persisted,..},..} if persisted == &vec![("level".into(),number(3.0))])));
}
#[test]
fn changed_persistent_fields_are_batched_then_saved_after_dispose() {
    let (mut c, id, key) = setup(CoreConfig::default());
    describe(&mut c, id, 0, vec![("level".into(), number(1.0))]);
    let out = reply(
        &mut c,
        id,
        10,
        vec![
            ("level".into(), number(2.0)),
            ("volatile".into(), number(9.0)),
        ],
    );
    assert!(saves(&out).is_empty());
    let (t, at) = timer(&out);
    assert_eq!(at, Timestamp(1010));
    c.handle(Timestamp(20), Input::SurfaceRemoved(SurfaceId(1)));
    assert!(c.snapshot().instances.is_empty());
    let out = c.handle(at, Input::Timer { id: t });
    assert_eq!(
        saves(&out),
        vec![(key, vec![("level".into(), number(2.0))])]
    );
    assert!(saves(&c.handle(at, Input::Timer { id: t })).is_empty());
}
#[test]
fn debounce_last_value_only_and_reversion_removes_dirty_field() {
    let (mut c, id, key) = setup(CoreConfig::default());
    describe(&mut c, id, 0, vec![("level".into(), number(1.0))]);
    let first = reply(&mut c, id, 10, vec![("level".into(), number(2.0))]);
    let (old, _) = timer(&first);
    // Init dispatches Shown, whose update returns level to the disk baseline.
    let reverted = reply(&mut c, id, 20, vec![("level".into(), number(1.0))]);
    assert!(
        reverted
            .iter()
            .any(|o| matches!(o,Output::ClearTimer {id} if *id==old))
    );
    assert!(saves(&c.handle(Timestamp(1010), Input::Timer { id: old })).is_empty());
    // A fresh generation reloads Init; the cache is still current.
    let mut cat = catalog();
    cat.plugins[0].revision = "2".into();
    c.handle(Timestamp(1020), Input::Catalog(cat));
    let new = c.snapshot().instances[0].id;
    describe(&mut c, new, 1020, vec![]);
    let first = reply(&mut c, new, 1030, vec![("level".into(), number(3.0))]);
    let (old, _) = timer(&first);
    let last = reply(&mut c, new, 1050, vec![("level".into(), number(4.0))]);
    let (t, at) = timer(&last);
    assert_ne!(t, old);
    assert_eq!(at, Timestamp(2050));
    assert!(saves(&c.handle(Timestamp(2030), Input::Timer { id: old })).is_empty());
    assert_eq!(
        saves(&c.handle(at, Input::Timer { id: t })),
        vec![(key, vec![("level".into(), number(4.0))])]
    );
}
#[test]
fn cache_wins_over_stale_disk_on_reload_and_surface_reconnect() {
    let (mut c, id, key) = setup(CoreConfig::default());
    describe(&mut c, id, 0, vec![]);
    reply(&mut c, id, 10, vec![("level".into(), number(7.0))]);
    let mut cat = catalog();
    cat.plugins[1].revision = "2".into();
    c.handle(Timestamp(20), Input::Catalog(cat));
    let new = c.snapshot().instances[0].id;
    assert_ne!(new, id);
    let out = describe(&mut c, new, 20, vec![("level".into(), number(1.0))]);
    assert!(out.iter().any(|o|matches!(o,Output::CallLogic {invocation:LogicCall::Init {persisted,..},..} if persisted==&vec![("level".into(),number(7.0))])));
    c.handle(Timestamp(30), Input::SurfaceRemoved(SurfaceId(1)));
    let out = c.handle(Timestamp(40), Input::SurfaceAdded(info(2)));
    let new = c.snapshot().instances[0].id;
    assert!(
        out.iter()
            .any(|o| matches!(o,Output::CreateInstance {persist_key,..} if persist_key==&key))
    );
    let out = describe(&mut c, new, 40, vec![]);
    assert!(out.iter().any(|o|matches!(o,Output::CallLogic {invocation:LogicCall::Init {persisted,..},..} if persisted==&vec![("level".into(),number(7.0))])));
}
#[test]
fn set_owner_and_surface_name_scope_profile_and_layer_independently() {
    let (mut c, _, key) = setup(CoreConfig::default());
    assert_eq!(key.plugin, "p".parse().unwrap());
    assert_eq!(
        key.scope,
        PersistScope::ProfileSlot {
            slot: "main".parse().unwrap(),
            set: "p/main".parse().unwrap()
        }
    );
    let out = c.handle(
        Timestamp(1),
        Input::Key {
            source: KeySource::Surface(SurfaceId(1)),
            code: WatchKey::Fn,
            phase: KeyPhase::Pressed,
        },
    );
    let layer_key = out
        .iter()
        .find_map(|o| {
            if let Output::CreateInstance {
                persist_key,
                plugin,
                ..
            } = o
            {
                assert_eq!(plugin, &"shared".parse().unwrap());
                Some(persist_key.clone())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(
        layer_key.scope,
        PersistScope::Layer("p/main".parse().unwrap())
    );
    assert_eq!(layer_key.plugin, key.plugin);
    assert_eq!(layer_key.surface, key.surface);
    assert_ne!(layer_key, key);
}
#[test]
fn unchanged_or_invalid_fields_do_not_schedule_save() {
    let (mut c, id, _) = setup(CoreConfig::default());
    describe(&mut c, id, 0, vec![("level".into(), number(1.0))]);
    let out = reply(
        &mut c,
        id,
        10,
        vec![
            ("level".into(), number(1.0)),
            ("volatile".into(), number(5.0)),
            ("text".into(), number(9.0)),
        ],
    );
    assert!(
        !out.iter()
            .any(|o| matches!(o, Output::SetTimer { .. } | Output::Persist { .. }))
    );
}

#[test]
fn invalid_description_does_not_seed_cache_before_catalog_retry() {
    let (mut c, id, _) = setup(CoreConfig::default());
    let bad = LogicDescription {
        state: vec![
            StateField {
                name: "level".into(),
                kind: FieldKind::Scalar(ScalarKind::Number),
                persist: true,
            },
            StateField {
                name: "level".into(),
                kind: FieldKind::Scalar(ScalarKind::Number),
                persist: true,
            },
        ],
        ..LogicDescription::default()
    };
    c.handle(
        Timestamp(1),
        Input::LogicDescribed {
            instance: id,
            description: bad,
            persisted: vec![],
        },
    );
    assert_eq!(c.snapshot().instances[0].status, InstanceStatus::Faulted);
    let mut cat = catalog();
    cat.plugins[0].revision = "2".into();
    c.handle(Timestamp(2), Input::Catalog(cat));
    let new = c.snapshot().instances[0].id;
    let out = describe(&mut c, new, 2, vec![("level".into(), number(8.0))]);
    assert!(out.iter().any(|o|matches!(o,Output::CallLogic {invocation:LogicCall::Init {persisted,..},..} if persisted==&vec![("level".into(),number(8.0))])));
}
#[test]
fn oversized_persistent_update_is_not_saved_or_reloaded() {
    let (mut c, id, _) = setup(CoreConfig {
        max_persist_bytes: 64,
        ..CoreConfig::default()
    });
    describe(&mut c, id, 0, vec![]);
    let out = reply(
        &mut c,
        id,
        10,
        vec![(
            "text".into(),
            StateValue::Scalar(Scalar::Text("x".repeat(100))),
        )],
    );
    assert!(
        out.iter().any(
            |o| matches!(o,Output::Log {event,..} if event.contains("persistent state exceeds"))
        )
    );
    assert!(!out.iter().any(|o| matches!(o, Output::SetTimer { .. })));
    let mut cat = catalog();
    cat.plugins[0].revision = "2".into();
    c.handle(Timestamp(20), Input::Catalog(cat));
    let new = c.snapshot().instances[0].id;
    let out = describe(&mut c, new, 20, vec![]);
    assert!(out.iter().any(|o|matches!(o,Output::CallLogic {invocation:LogicCall::Init {persisted,..},..} if persisted.is_empty())));
}
#[test]
fn loaded_bytes_limit_and_duplicate_names_are_enforced() {
    let (mut c, id, _) = setup(CoreConfig {
        max_persist_bytes: 64,
        ..CoreConfig::default()
    });
    let out = describe(
        &mut c,
        id,
        0,
        vec![
            ("level".into(), number(1.0)),
            ("level".into(), number(2.0)),
            (
                "text".into(),
                StateValue::Scalar(Scalar::Text("x".repeat(100))),
            ),
        ],
    );
    assert!(out.iter().any(|o|matches!(o,Output::CallLogic {invocation:LogicCall::Init {persisted,..},..} if persisted==&vec![("level".into(),number(2.0))])));
}

#[test]
fn different_sets_in_same_profile_slot_never_share_persisted_fields() {
    let (mut c, volume, volume_key) = setup(CoreConfig::default());
    describe(&mut c, volume, 0, vec![]);
    reply(&mut c, volume, 0, vec![("level".into(), number(7.0))]);
    reply(&mut c, volume, 0, vec![]);
    let mut cat = catalog();
    let mut brightness = cat.sets[0].clone();
    brightness.reference = "p/brightness".parse().unwrap();
    cat.sets.push(brightness);
    let mut browser = ProfileSpec::default_for("p/browser".parse().unwrap());
    browser.for_target = Some(ProfileTarget::App("browser".into()));
    browser.slots.push(SlotSpec {
        id: "main".parse().unwrap(),
        candidates: vec!["p/brightness".parse().unwrap()],
    });
    cat.profiles.push(browser);
    c.handle(Timestamp(1), Input::Catalog(cat));
    let out = c.handle(
        Timestamp(2),
        Input::Fact {
            name: "app.id".into(),
            value: Some(FactValue::Text("browser".into())),
        },
    );
    let brightness_key = out
        .iter()
        .find_map(|o| match o {
            Output::CreateInstance { persist_key, .. } => Some(persist_key),
            _ => None,
        })
        .unwrap();
    assert_ne!(&volume_key, brightness_key);
    let brightness = c.snapshot().instances[0].id;
    let init = describe(&mut c, brightness, 2, vec![]);
    assert!(
        init.iter().any(|o| matches!(o, Output::CallLogic {
        invocation: LogicCall::Init { persisted, .. }, ..
    } if persisted.is_empty())),
        "{init:?}"
    );
    reply(&mut c, brightness, 2, vec![("level".into(), number(0.25))]);
    reply(&mut c, brightness, 2, vec![]);
    c = Core::restore(c.checkpoint()).unwrap();
    c.handle(
        Timestamp(3),
        Input::Fact {
            name: "app.id".into(),
            value: None,
        },
    );
    let volume = c.snapshot().instances[0].id;
    let init = describe(&mut c, volume, 3, vec![]);
    assert!(
        init.iter().any(|o| matches!(o, Output::CallLogic {
        invocation: LogicCall::Init { persisted, .. }, ..
    } if persisted == &vec![("level".into(), number(7.0))])),
        "{init:?}"
    );
}

#[test]
fn oversized_restored_init_resets_whole_state_and_retains_config() {
    let mut c = Core::new(CoreConfig::default());
    let mut cat = catalog();
    cat.sets[0].logic_config = "x".repeat(256 * 1024 - 100);
    c.handle(Timestamp(0), Input::Catalog(cat.clone()));
    let out = c.handle(Timestamp(0), Input::SurfaceAdded(info(1)));
    let id = out
        .iter()
        .find_map(|o| match o {
            Output::CreateInstance { instance, .. } => Some(*instance),
            _ => None,
        })
        .unwrap();
    let out = describe(
        &mut c,
        id,
        0,
        vec![
            ("level".into(), number(1.0)),
            (
                "text".into(),
                StateValue::Scalar(Scalar::Text("restored".into())),
            ),
        ],
    );
    assert!(out.iter().any(|o| matches!(o, Output::CallLogic {invocation: LogicCall::Init {config, persisted}, ..} if config == &cat.sets[0].logic_config && persisted.is_empty())));
    assert!(
        out.iter()
            .any(|o| matches!(o, Output::Log {event, ..} if event.contains("restored state")))
    );
}
#[test]
fn catalog_config_budget_includes_init_overhead_and_preserves_previous_catalog() {
    let mut c = Core::new(CoreConfig {
        max_update_bytes: 4096,
        ..CoreConfig::default()
    });
    c.handle(Timestamp(0), Input::Catalog(catalog()));
    let mut cat = catalog();
    cat.sets[0].logic_config = "x".repeat(4096);
    let out = c.handle(Timestamp(1), Input::Catalog(cat));
    assert!(
        out.iter()
            .any(|o| matches!(o, Output::Log {event, ..} if event.contains("configuration")))
    );
    let out = c.handle(Timestamp(2), Input::SurfaceAdded(info(1)));
    let id = out
        .iter()
        .find_map(|o| match o {
            Output::CreateInstance { instance, .. } => Some(*instance),
            _ => None,
        })
        .unwrap();
    let out = describe(&mut c, id, 2, vec![]);
    assert!(out.iter().any(|o| matches!(o, Output::CallLogic {invocation:LogicCall::Init {config,..}, ..} if config.is_empty())));
}
