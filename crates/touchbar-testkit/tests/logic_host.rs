use touchbar_core::*;
use touchbar_model::*;
use touchbar_testkit::*;

#[test]
fn fake_selected_binding_is_boolean_and_nested_button_is_hittable() {
    let mut h = Harness::new(CoreConfig::default());
    let c = catalog()
        .plugin("fake")
        .set("main", |s| {
            let mut button = WidgetSpec::new("outer".parse().unwrap(), WidgetKind::Button);
            button
                .state_bindings
                .insert(StateBinding::Selected, "selected".into());
            button.handlers.insert(ActionKind::Activated, "go".into());
            button.children.push(WidgetSpec::new(
                "label".parse().unwrap(),
                WidgetKind::Noninteractive,
            ));
            s.widget(button);
        })
        .profile("default", |p| {
            p.default().slot("main", &["fake/main"]);
        })
        .build();
    h.send(Input::Catalog(c));
    h.send(Input::SurfaceAdded(headless(1)));
    assert_eq!(h.snapshot().instances[0].status, InstanceStatus::Visible);
    h.tap(SurfaceId(1), "outer");
    assert!(h.logic.calls.iter().any(
        |(_, _, call)| matches!(call,LogicCall::Handle(LogicEvent::Action(e)) if e.action=="go")
    ));
}

#[test]
fn registered_wasm_executes_and_other_plugins_use_fake_logic() {
    let mut logic = WasmLogic::new();
    logic
        .register("echo", "test-1", touchbar_wasm_fixtures::ECHO)
        .unwrap();
    logic.fallback.register("fake/main", None, |_, _| {
        LogicReaction::Update(LogicUpdate {
            state: vec![(
                "label".into(),
                StateValue::Scalar(Scalar::Text("fake".into())),
            )],
            ..Default::default()
        })
    });
    let c = catalog()
        .plugin("echo")
        .set("main", |s| {
            s.spec.widgets.push(WidgetSpec::new(
                "message".parse().unwrap(),
                WidgetKind::Noninteractive,
            ));
        })
        .plugin("fake")
        .set("main", |s| {
            s.spec.widgets.push(WidgetSpec::new(
                "label".parse().unwrap(),
                WidgetKind::Noninteractive,
            ));
        })
        .profile("default", |p| {
            p.default()
                .slot("wasm", &["echo/main"])
                .slot("fake", &["fake/main"]);
        })
        .build();
    let mut h = Harness::with_logic(CoreConfig::default(), logic);
    h.send(Input::Catalog(c));
    h.send(Input::SurfaceAdded(headless(1)));
    let snapshot = h.snapshot();
    let echo = snapshot
        .instances
        .iter()
        .find(|i| i.set.plugin.as_str() == "echo")
        .unwrap();
    assert!(
        echo.state.iter().any(|(name, v)| name == "last"
            && matches!(v,StateValue::Scalar(Scalar::Text(s)) if s.contains("now_ms"))),
        "{:?}",
        echo.state
    );
    assert!(
        h.logic
            .fallback
            .calls
            .iter()
            .all(|(id, _, _)| *id != echo.id)
    );
}
#[test]
fn real_hang_fault_is_delivered_at_current_virtual_time() {
    let mut logic = WasmLogic::new();
    logic
        .register("hang", "test-1", touchbar_wasm_fixtures::HANG)
        .unwrap();
    let c = catalog()
        .plugin("hang")
        .set("main", |s| {
            s.button("hang");
        })
        .profile("default", |p| {
            p.default().slot("main", &["hang/main"]);
        })
        .build();
    let mut h = Harness::with_logic(CoreConfig::default(), logic);
    h.send(Input::Catalog(c));
    h.send(Input::SurfaceAdded(headless(1)));
    h.tap(SurfaceId(1), "hang");
    assert_eq!(h.now(), Timestamp(0));
    assert!(h.trace.iter().any(|e| e.at == Timestamp(0)
        && matches!(&e.input,Input::LogicFault{reason,..} if reason.contains("timeout"))));
}

#[test]
fn wasm_drag_persists_after_one_second_and_new_generation_reads_same_store() {
    let mut logic = WasmLogic::new();
    logic
        .register("persist", "test-1", touchbar_wasm_fixtures::PERSIST)
        .unwrap();
    let c = catalog()
        .plugin("persist")
        .set("main", |s| {
            let mut slider = WidgetSpec::new(
                "level".parse().unwrap(),
                WidgetKind::Slider {
                    minimum: 0.0,
                    maximum: 1.0,
                },
            );
            slider
                .handlers
                .insert(ActionKind::ValueChanged, "change".into());
            slider
                .state_bindings
                .insert(StateBinding::Value, "value".into());
            s.widget(slider);
            s.spec.persist = vec!["value".into()];
        })
        .profile("default", |p| {
            p.default().slot("main", &["persist/main"]);
        })
        .build();
    let mut h = Harness::with_logic(CoreConfig::default(), logic);
    h.enable_invariant_checks();
    h.send(Input::Catalog(c.clone()));
    h.send(Input::SurfaceAdded(headless(1)));
    let first = h.snapshot().instances[0].clone();
    h.drag(SurfaceId(1), "level", 0.1, 0.7, 300, 10);
    assert_eq!(h.now(), Timestamp(300));
    let value = h.snapshot().instances[0]
        .state
        .iter()
        .find(|(n, _)| n == "value")
        .unwrap()
        .1
        .clone();
    assert!(matches!(value,StateValue::Scalar(Scalar::Number(v)) if (v-0.7).abs()<0.00001));
    h.advance_ms(999);
    assert!(
        !h.trace
            .iter()
            .flat_map(|e| &e.outputs)
            .any(|o| matches!(o, Output::Persist { .. }))
    );
    h.advance_ms(1);
    let stored = h.logic.persisted(&first.persist_key);
    assert!(stored.contains(&("value".into(), value)));
    assert!(stored.iter().any(|(n, _)| n == "label"));
    h.logic
        .register("persist", "test-2", touchbar_wasm_fixtures::PERSIST)
        .unwrap();
    let mut next = c;
    next.plugins[0].revision = "test-2".into();
    h.send(Input::Catalog(next));
    let second = &h.snapshot().instances[0];
    assert_ne!(first.id, second.id);
    assert_eq!(
        second.state.iter().find(|(n, _)| n == "value").unwrap(),
        stored.iter().find(|(n, _)| n == "value").unwrap()
    );
    h.send(Input::SurfaceRemoved(SurfaceId(1)));
    h.finish_invariant_checks().unwrap();
    assert!(h.host_errors.is_empty());
}
#[test]
fn subscriptions_are_delivered_to_selected_set_and_tag_on_selected_surface() {
    let mut logic = WasmLogic::new();
    logic
        .register("echo", "test-1", touchbar_wasm_fixtures::ECHO)
        .unwrap();
    let initial = LogicUpdate {
        subscriptions: Some(vec![SubscriptionSpec {
            tag: "sink".into(),
            kind: SubscriptionKind::AudioWatch {
                target: AudioTarget::DefaultSink,
            },
            background: false,
        }]),
        ..Default::default()
    };
    let c = catalog()
        .plugin("echo")
        .set("main", |s| {
            s.spec.logic_config = serde_json::to_string(&initial).unwrap();
        })
        .profile("default", |p| {
            p.default().slot("main", &["echo/main"]);
        })
        .build();
    let mut h = Harness::with_logic(CoreConfig::default(), logic);
    h.send(Input::Catalog(c));
    h.send(Input::SurfaceAdded(headless(1)));
    h.send(Input::SurfaceAdded(headless(2)));
    h.subscription_data(
        SurfaceId(2),
        "echo/main",
        "sink",
        SubscriptionData::Audio(AudioState {
            available: true,
            volume: 0.4,
            muted: true,
            name: "second".into(),
        }),
    );
    let snapshot = h.snapshot();
    for instance in snapshot.instances {
        let StateValue::Scalar(Scalar::Text(last)) = &instance.state[0].1 else {
            panic!("last")
        };
        assert_eq!(last.contains("second"), instance.surface == SurfaceId(2));
    }
}
#[test]
fn loading_new_revision_does_not_change_old_catalog_logic() {
    let mut logic = WasmLogic::new();
    logic
        .register("owner", "test-1", touchbar_wasm_fixtures::ECHO)
        .unwrap();
    logic
        .register("owner", "test-2", touchbar_wasm_fixtures::PERSIST)
        .unwrap();
    let c = catalog()
        .plugin("owner")
        .set("main", |_| {})
        .profile("default", |p| {
            p.default().slot("main", &["owner/main"]);
        })
        .build();
    let mut h = Harness::with_logic(CoreConfig::default(), logic);
    h.send(Input::Catalog(c));
    h.send(Input::SurfaceAdded(headless(1)));
    assert!(
        h.snapshot().instances[0]
            .state
            .iter()
            .any(|(n, _)| n == "last")
    );
}
#[test]
fn failing_registered_module_is_not_replaced_with_fake_success() {
    let mut logic = WasmLogic::new();
    assert!(
        logic
            .register("owner", "test-1", touchbar_wasm_fixtures::WASI)
            .is_err()
    );
    let c = catalog()
        .plugin("owner")
        .set("main", |_| {})
        .profile("default", |p| {
            p.default().slot("main", &["owner/main"]);
        })
        .build();
    let mut h = Harness::with_logic(CoreConfig::default(), logic);
    h.send(Input::Catalog(c));
    h.send(Input::SurfaceAdded(headless(1)));
    assert_eq!(h.snapshot().instances[0].status, InstanceStatus::Faulted);
    assert!(h.logic.fallback.calls.is_empty());
}

#[test]
fn late_call_after_wasm_dispose_is_noop_not_fake_fallback() {
    let mut host = WasmLogic::new();
    host.register("echo", "test-1", touchbar_wasm_fixtures::ECHO)
        .unwrap();
    let set = SetSpec::default_for("echo/main".parse().unwrap());
    let provider = PluginSpec {
        id: "echo".parse().unwrap(),
        revision: "test-1".into(),
        enabled: true,
        has_logic: true,
        requires: vec![],
    };
    let key = PersistKey {
        plugin: provider.id.clone(),
        surface: "headless:1".parse().unwrap(),
        scope: PersistScope::Layer(set.reference.clone()),
    };
    let instance = InstanceId {
        number: 1,
        generation: Generation(1),
    };
    host.create(Timestamp(0), instance, &set, Some(&provider), &key);
    host.dispose(instance);
    assert!(
        host.invoke(
            Timestamp(0),
            &set.reference,
            instance,
            CallId(1),
            &LogicCall::Handle(LogicEvent::Shown)
        )
        .is_none()
    );
    assert!(host.fallback.calls.is_empty());
}

#[test]
fn scenario_dsl_accepts_wasm_host_and_routes_drag_and_subscription_steps() {
    let mut logic = WasmLogic::new();
    logic
        .register("persist", "test-1", touchbar_wasm_fixtures::PERSIST)
        .unwrap();
    let c = catalog()
        .plugin("persist")
        .set("main", |s| {
            s.slider("level");
            s.spec.widgets[0]
                .handlers
                .insert(ActionKind::ValueChanged, "change".into());
        })
        .profile("default", |p| {
            p.default().slot("main", &["persist/main"]);
        })
        .build();
    let h = scenario()
        .logic(logic)
        .catalog(c)
        .drag("level", 0.2, 0.6, 200, 5)
        .run();
    assert_eq!(h.now(), Timestamp(200));
    assert!(
        h.snapshot().instances[0]
            .state
            .iter()
            .any(|(name, value)| name == "value"
                && matches!(value,StateValue::Scalar(Scalar::Number(v)) if (v-0.6).abs()<0.00001))
    );
    let mut logic = WasmLogic::new();
    logic
        .register("echo", "test-1", touchbar_wasm_fixtures::ECHO)
        .unwrap();
    let init = LogicUpdate {
        subscriptions: Some(vec![SubscriptionSpec {
            tag: "sink".into(),
            kind: SubscriptionKind::AudioWatch {
                target: AudioTarget::DefaultSink,
            },
            background: false,
        }]),
        ..Default::default()
    };
    let c = catalog()
        .plugin("echo")
        .set("main", |s| {
            s.spec.logic_config = serde_json::to_string(&init).unwrap();
        })
        .profile("default", |p| {
            p.default().slot("main", &["echo/main"]);
        })
        .build();
    let h = scenario()
        .logic(logic)
        .catalog(c)
        .subscription_data(
            "echo/main",
            "sink",
            SubscriptionData::Audio(AudioState {
                available: true,
                volume: 0.4,
                muted: false,
                name: "dsl".into(),
            }),
        )
        .run();
    assert!(h.snapshot().instances[0].state.iter().any(
        |(_, value)| matches!(value,StateValue::Scalar(Scalar::Text(s)) if s.contains("dsl"))
    ));
}
