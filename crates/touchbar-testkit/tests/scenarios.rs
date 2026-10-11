use std::collections::BTreeMap;
use touchbar_core::*;
use touchbar_model::*;
use touchbar_testkit::*;

fn info(id: u64) -> SurfaceInfo {
    headless(id)
}
fn setup(catalog: Catalog, logic: FakeLogic, config: CoreConfig, infos: &[SurfaceInfo]) -> Harness {
    let mut h = Harness::new(config);
    h.enable_invariant_checks();
    h.logic = logic;
    h.send(Input::Catalog(catalog));
    for info in infos {
        h.send(Input::SurfaceAdded(info.clone()));
    }
    h
}
fn ready() -> Harness {
    setup(
        fixtures::controls_with_panel(),
        fixtures::controls_logic(),
        CoreConfig::default(),
        &[info(1)],
    )
}
fn fn_key(h: &mut Harness, id: u64, pressed: bool) {
    h.send(Input::Key {
        source: KeySource::Surface(SurfaceId(id)),
        code: WatchKey::Fn,
        phase: if pressed {
            KeyPhase::Pressed
        } else {
            KeyPhase::Released
        },
    });
}
fn app(h: &mut Harness) {
    h.send(Input::Fact {
        name: "app.id".into(),
        value: Some(FactValue::Text("editor".into())),
    });
}
fn alternate_catalog() -> Catalog {
    let mut c = fixtures::controls_with_panel();
    let mut set = c.sets[0].clone();
    set.reference = "controls/alternate".parse().unwrap();
    c.sets.push(set);
    let mut p = ProfileSpec::default_for("controls/editor".parse().unwrap());
    p.for_target = Some(ProfileTarget::App("editor".into()));
    p.slots = vec![SlotSpec {
        id: "main".parse().unwrap(),
        candidates: vec!["controls/alternate".parse().unwrap()],
    }];
    c.profiles.push(p);
    c
}
fn checked(mut h: Harness) {
    for s in h.snapshot().surfaces {
        h.send(Input::SurfaceRemoved(s.info.id));
    }
    h.advance_ms(10000);
    assert!(h.invariant_error.is_none(), "{:?}", h.invariant_error);
    h.finish_invariant_checks().unwrap();
    assert_eq!(
        ReplayDump::from_json(&h.dump().to_json().unwrap())
            .unwrap()
            .replay()
            .unwrap()
            .core
            .snapshot(),
        h.snapshot()
    );
}
fn touch(h: &mut Harness, id: u64, contact: u32, phase: TouchPhase, point: Point) -> Vec<Output> {
    let start = h.trace.len();
    h.touch(SurfaceId(id), contact, phase, point);
    h.trace[start..]
        .iter()
        .flat_map(|e| e.outputs.clone())
        .collect()
}

#[test]
fn mic_toggle_survives_panel_close() {
    let mut h = setup(
        fixtures::controls_with_panel(),
        fixtures::controls_logic(),
        CoreConfig {
            effect_slots: 1,
            ..CoreConfig::default()
        },
        &[info(1)],
    );
    h.effects.delay_ms = 1000;
    h.tap(SurfaceId(1), "microphone");
    h.touch(
        SurfaceId(1),
        1,
        TouchPhase::Down,
        h.hit(SurfaceId(1), "microphone"),
    );
    h.advance_ms(100);
    assert!(h.snapshot().surfaces[0].presentation.is_some());
    let choose = h.hit(SurfaceId(1), "choose");
    touch(&mut h, 1, 2, TouchPhase::Down, choose);
    touch(&mut h, 1, 2, TouchPhase::Up, choose);
    assert!(h.snapshot().surfaces[0].presentation.is_none());
    assert_eq!(h.snapshot().requests.len(), 2);
    assert_eq!(h.effects.started.len(), 1);
    h.advance_ms(2000);
    assert_eq!(h.effects.started.len(), 2);
    assert!(h.effects.started.iter().all(|(_, e)| matches!(
        e,
        Effect::AudioSetMute {
            target: AudioTarget::DefaultSource,
            ..
        }
    )));
    assert!(h.effects.cancelled.is_empty());
    checked(h);
}
#[test]
fn hidden_instance_has_no_watchers() {
    let mut logic = FakeLogic::default();
    logic.register("controls/main", None, |_, _| {
        LogicReaction::Update(LogicUpdate {
            subscriptions: Some(vec![SubscriptionSpec {
                tag: "audio".into(),
                kind: SubscriptionKind::AudioWatch {
                    target: AudioTarget::DefaultSink,
                },
                background: false,
            }]),
            ..LogicUpdate::default()
        })
    });
    let mut surface = info(1);
    surface.caps.modes = true;
    let h = setup(
        fixtures::controls_with_panel(),
        logic,
        CoreConfig::default(),
        &[surface],
    );
    assert_eq!(h.snapshot().instances[0].status, InstanceStatus::Hidden);
    assert!(
        h.snapshot()
            .subscriptions
            .iter()
            .all(|s| s.active.is_none())
    );
    assert!(!h.trace.iter().any(|e| {
        e.outputs
            .iter()
            .any(|o| matches!(o, Output::Subscribe { .. }))
    }));
    checked(h);
}
#[test]
fn synthetic_cancel_cannot_open_panel() {
    let mut c = fixtures::controls_with_panel();
    c.sets[0].widgets[1]
        .handlers
        .insert(ActionKind::Cancelled, "cancel".into());
    let mut logic = FakeLogic::default();
    logic.register("controls/main", None, |_, call| {
        let mut update = LogicUpdate::default();
        if let LogicCall::Handle(LogicEvent::Action(ActionEvent {
            kind: ActionKind::Cancelled,
            origin: Origin::Synthetic,
            ..
        })) = call
        {
            update.presentation = Some(PresentationCommand::Open {
                panel: "devices".parse().unwrap(),
                placement: Placement::Full,
                lifecycle: PresentationLifecycle::Persistent,
            });
        }
        LogicReaction::Update(update)
    });
    let mut h = setup(c, logic, CoreConfig::default(), &[info(1)]);
    h.touch(
        SurfaceId(1),
        1,
        TouchPhase::Down,
        h.hit(SurfaceId(1), "microphone"),
    );
    let mut layout = h
        .trace
        .iter()
        .rev()
        .find(|e| matches!(e.input, Input::Layout { .. }))
        .unwrap()
        .input
        .clone();
    if let Input::Layout { hit_map, .. } = &mut layout {
        hit_map.retain(|hit| hit.widget.as_str() != "microphone");
    }
    h.send(layout);
    assert!(h.logic.calls.iter().any(|(_, _, c)| matches!(
        c,
        LogicCall::Handle(LogicEvent::Action(ActionEvent {
            kind: ActionKind::Cancelled,
            origin: Origin::Synthetic,
            ..
        }))
    )));
    assert!(h.snapshot().surfaces[0].presentation.is_none());
    assert_eq!(h.snapshot().instances[0].status, InstanceStatus::Visible);
    checked(h);
}
#[test]
fn probe_state_resets_on_hide() {
    let mut c = fixtures::controls_with_panel();
    let w = &mut c.sets[0].widgets[1];
    w.handlers.insert(ActionKind::Pressed, "probe".into());
    w.handlers.insert(ActionKind::Cancelled, "probe".into());
    w.state_bindings.insert(StateBinding::Text, "phase".into());
    let mut logic = FakeLogic::default();
    logic.register("controls/main", None, |_, call| {
        let mut update = LogicUpdate::default();
        match call {
            LogicCall::Init { .. } | LogicCall::Handle(LogicEvent::Hidden) => {
                update.state = vec![(
                    "phase".into(),
                    StateValue::Scalar(Scalar::Text("idle".into())),
                )];
                update.cancel = vec!["probe".into()];
                update.subscriptions = Some(vec![]);
            }
            LogicCall::Handle(LogicEvent::Action(ActionEvent {
                kind: ActionKind::Pressed,
                ..
            })) => {
                update.state = vec![(
                    "phase".into(),
                    StateValue::Scalar(Scalar::Text("probing".into())),
                )];
                update.requests = vec![EffectRequest {
                    tag: "probe".into(),
                    effect: Effect::TimerAfter { ms: 5000 },
                    lifetime: Some(Lifetime::UntilHidden),
                    coalesce: None,
                    timeout_ms: None,
                }];
                update.subscriptions = Some(vec![SubscriptionSpec {
                    tag: "probe".into(),
                    kind: SubscriptionKind::AudioWatch {
                        target: AudioTarget::DefaultSource,
                    },
                    background: false,
                }]);
            }
            _ => {}
        }
        LogicReaction::Update(update)
    });
    let mut surface = info(1);
    surface.caps.modes = true;
    let mut h = setup(c, logic, CoreConfig::default(), &[surface]);
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Display,
    });
    h.touch(
        SurfaceId(1),
        1,
        TouchPhase::Down,
        h.hit(SurfaceId(1), "microphone"),
    );
    let owner = h.snapshot().instances[0].id;
    assert_eq!(h.snapshot().requests.len(), 1);
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Native,
    });
    let snap = h.snapshot();
    let old = snap.instances.iter().find(|i| i.id == owner).unwrap();
    assert_eq!(old.status, InstanceStatus::Hidden);
    assert!(old.requests.is_empty());
    assert!(old.subscriptions.is_empty());
    assert_eq!(
        old.state,
        vec![(
            "phase".into(),
            StateValue::Scalar(Scalar::Text("idle".into()))
        )]
    );
    assert!(
        h.logic
            .calls
            .iter()
            .any(|(id, _, call)| *id == owner
                && matches!(call, LogicCall::Handle(LogicEvent::Hidden)))
    );
    assert!(h.logic.calls.iter().any(|(id, _, call)| *id == owner
        && matches!(
            call,
            LogicCall::Handle(LogicEvent::Action(ActionEvent {
                kind: ActionKind::Cancelled,
                ..
            }))
        )));
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Display,
    });
    assert!(h.snapshot().requests.is_empty());
    checked(h);
}
#[test]
fn contact_outside_panel_finishes() {
    let mut h = setup(
        alternate_catalog(),
        fixtures::controls_logic(),
        CoreConfig::default(),
        &[info(1)],
    );
    let outside = h.hit(SurfaceId(1), "volume");
    h.touch(SurfaceId(1), 1, TouchPhase::Down, outside);
    let original = h.snapshot().captures[0].clone();
    h.touch(
        SurfaceId(1),
        2,
        TouchPhase::Down,
        h.hit(SurfaceId(1), "microphone"),
    );
    h.advance_ms(100);
    assert!(h.snapshot().surfaces[0].presentation.is_some());
    app(&mut h);
    let out = touch(&mut h, 1, 1, TouchPhase::Up, outside);
    assert!(out.contains(&Output::CaptureFinished {
        capture: original,
        completion: CaptureCompletion::Up
    }));
    assert_eq!(h.snapshot().captures.len(), 1);
    assert!(h.snapshot().surfaces[0].profile.pending.is_some());
    h.touch_up(SurfaceId(1), 2);
    assert!(h.snapshot().captures.is_empty());
    assert_eq!(
        h.snapshot().surfaces[0]
            .profile
            .active
            .as_ref()
            .unwrap()
            .to_string(),
        "controls/editor"
    );
    checked(h);
}
#[test]
fn first_contact_after_resume_holds() {
    let mut surface = info(1);
    surface.caps.modes = true;
    let mut h = setup(
        fixtures::controls_with_panel(),
        FakeLogic::default(),
        CoreConfig::default(),
        &[surface],
    );
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Display,
    });
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Native,
    });
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Display,
    });
    h.touch(
        SurfaceId(1),
        1,
        TouchPhase::Down,
        h.hit(SurfaceId(1), "microphone"),
    );
    h.advance_ms(100);
    assert!(h.snapshot().surfaces[0].presentation.is_some());
    assert_eq!(h.snapshot().captures[0].contact, ContactId::new(1).unwrap());
    checked(h);
}
#[test]
fn fn_layer_round_trip() {
    let mut h = ready();
    let owner = h.snapshot().instances[0].id;
    h.touch(
        SurfaceId(1),
        1,
        TouchPhase::Down,
        h.hit(SurfaceId(1), "microphone"),
    );
    fn_key(&mut h, 1, true);
    assert!(h.snapshot().captures.is_empty());
    assert_eq!(h.snapshot().surfaces[0].layers.len(), 1);
    assert_eq!(
        h.snapshot()
            .instances
            .iter()
            .find(|i| i.id == owner)
            .unwrap()
            .status,
        InstanceStatus::Visible
    );
    fn_key(&mut h, 1, false);
    assert!(h.snapshot().surfaces[0].layers.is_empty());
    assert_eq!(h.snapshot().instances[0].id, owner);
    assert_eq!(h.snapshot().instances[0].status, InstanceStatus::Visible);
    checked(h);
}
#[test]
fn profile_pending_deadline() {
    let mut h = setup(
        alternate_catalog(),
        FakeLogic::default(),
        CoreConfig::default(),
        &[info(1)],
    );
    h.touch(
        SurfaceId(1),
        1,
        TouchPhase::Down,
        h.hit(SurfaceId(1), "microphone"),
    );
    app(&mut h);
    let pending = h.snapshot().surfaces[0].profile.pending.clone().unwrap();
    assert_eq!(pending.deadline, Timestamp(1500));
    h.advance_ms(1499);
    assert_eq!(h.snapshot().captures.len(), 1);
    assert!(h.snapshot().surfaces[0].profile.pending.is_some());
    h.advance_ms(1);
    assert!(h.snapshot().captures.is_empty());
    assert!(h.snapshot().surfaces[0].profile.pending.is_none());
    assert_eq!(
        h.snapshot().surfaces[0]
            .profile
            .active
            .as_ref()
            .unwrap()
            .to_string(),
        "controls/editor"
    );
    checked(h);
}
#[test]
fn slider_coalesces_writes() {
    for delay in [1000, 100] {
        let mut h = ready();
        h.effects.delay_ms = delay;
        h.touch(SurfaceId(1), 1, TouchPhase::Down, Point { x: 0.0, y: 25.0 });
        for n in 1..50 {
            h.advance_ms(20);
            h.touch(
                SurfaceId(1),
                1,
                TouchPhase::Move,
                Point {
                    x: n as f32 * 80.0 / 49.0,
                    y: 25.0,
                },
            );
        }
        assert_eq!(
            h.logic
                .calls
                .iter()
                .filter(|(_, _, call)| matches!(
                    call,
                    LogicCall::Handle(LogicEvent::Action(ActionEvent {
                        kind: ActionKind::ValueChanged,
                        ..
                    }))
                ))
                .count(),
            50
        );
        h.advance_ms(20);
        h.touch_up(SurfaceId(1), 1);
        h.advance_ms(2000);
        let values: Vec<_> = h
            .effects
            .started
            .iter()
            .filter_map(|(_, e)| {
                if let Effect::AudioSetVolume { level, .. } = e {
                    Some(*level)
                } else {
                    None
                }
            })
            .collect();
        if delay == 1000 {
            assert_eq!(values.len(), 2);
        } else {
            assert!(values.len() <= 11, "{values:?}");
        }
        assert_eq!(values.first(), Some(&0.0));
        assert_eq!(values.last(), Some(&1.0));
        assert!(h.effects.cancelled.is_empty());
        checked(h);
    }
}
#[test]
fn committed_cancel_and_replace() {
    let mut c = fixtures::controls_with_panel();
    let mut cancel = WidgetSpec::new("cancel".parse().unwrap(), WidgetKind::Button);
    cancel
        .handlers
        .insert(ActionKind::Activated, "cancel".into());
    c.sets[0].widgets.push(cancel);
    let mut logic = FakeLogic::default();
    logic.register("controls/main", None, |_, call| {
        let mut update = LogicUpdate::default();
        if let LogicCall::Handle(LogicEvent::Action(a)) = call {
            if a.action == "cancel" {
                update.cancel.push("volume".into());
            } else if let Some(Scalar::Number(level)) = a.value {
                update.requests.push(EffectRequest {
                    tag: "volume".into(),
                    effect: Effect::AudioSetVolume {
                        target: AudioTarget::DefaultSink,
                        level,
                    },
                    lifetime: Some(Lifetime::Committed),
                    coalesce: Some("volume".into()),
                    timeout_ms: None,
                });
            }
        }
        LogicReaction::Update(update)
    });
    let mut h = setup(c, logic, CoreConfig::default(), &[info(1)]);
    h.effects.delay_ms = 500;
    h.touch(SurfaceId(1), 1, TouchPhase::Down, Point { x: 0.0, y: 25.0 });
    let running = h.effects.started[0].0;
    h.touch(
        SurfaceId(1),
        1,
        TouchPhase::Move,
        Point { x: 40.0, y: 25.0 },
    );
    h.touch(
        SurfaceId(1),
        1,
        TouchPhase::Move,
        Point { x: 80.0, y: 25.0 },
    );
    assert_eq!(h.snapshot().requests.len(), 2);
    assert_eq!(h.effects.started.len(), 1);
    assert!(matches!(
        h.snapshot()
            .requests
            .iter()
            .find(|r| !r.running)
            .unwrap()
            .effect,
        Effect::AudioSetVolume { level: 1.0, .. }
    ));
    let point = h.hit(SurfaceId(1), "cancel");
    touch(&mut h, 1, 2, TouchPhase::Down, point);
    touch(&mut h, 1, 2, TouchPhase::Up, point);
    assert!(h.effects.cancelled.contains(&running));
    assert!(h.snapshot().requests.is_empty());
    checked(h);

    let mut surface = info(1);
    surface.caps.modes = true;
    let mut h = setup(
        fixtures::controls_with_panel(),
        fixtures::controls_logic(),
        CoreConfig::default(),
        &[surface],
    );
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Display,
    });
    h.effects.delay_ms = 500;
    h.tap(SurfaceId(1), "microphone");
    let running = h.effects.started[0].0;
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Native,
    });
    assert!(h.snapshot().requests.iter().any(|r| r.id == running));
    h.send(Input::SurfaceRemoved(SurfaceId(1)));
    assert!(h.snapshot().requests.iter().any(|r| r.id == running));
    assert!(!h.effects.cancelled.contains(&running));
    h.advance_ms(500);
    assert!(h.snapshot().requests.is_empty());
    checked(h);
}
#[test]
fn committed_spawn_survives_dispose() {
    let mut logic = FakeLogic::default();
    logic.register("controls/main", None, |_, call| {
        let mut u = LogicUpdate::default();
        if matches!(call, LogicCall::Handle(LogicEvent::Shown)) {
            u.requests = vec![
                EffectRequest {
                    tag: "committed".into(),
                    effect: Effect::SpawnRun(SpawnSpec::new(vec!["fake-committed".into()])),
                    lifetime: Some(Lifetime::Committed),
                    coalesce: None,
                    timeout_ms: Some(5000),
                },
                EffectRequest {
                    tag: "transient".into(),
                    effect: Effect::SpawnRun(SpawnSpec::new(vec!["fake-transient".into()])),
                    lifetime: Some(Lifetime::UntilHidden),
                    coalesce: None,
                    timeout_ms: Some(5000),
                },
            ];
            u.subscriptions = Some(vec![SubscriptionSpec {
                tag: "lines".into(),
                kind: SubscriptionKind::SpawnLines(SpawnSpec::new(vec!["fake-lines".into()])),
                background: false,
            }]);
        }
        LogicReaction::Update(u)
    });
    let mut h = Harness::new(CoreConfig::default());
    h.enable_invariant_checks();
    h.effects.delay_ms = 1000;
    h.logic = logic;
    h.send(Input::Catalog(fixtures::controls_with_panel()));
    h.send(Input::SurfaceAdded(info(1)));
    let owner = h.snapshot().instances[0].id;
    assert_eq!(h.effects.started.len(), 2);
    let committed = h
        .effects
        .started
        .iter()
        .find(|(_, e)| matches!(e,Effect::SpawnRun(s) if s.argv[0]=="fake-committed"))
        .unwrap()
        .0;
    let transient = h
        .effects
        .started
        .iter()
        .find(|(_, e)| matches!(e,Effect::SpawnRun(s) if s.argv[0]=="fake-transient"))
        .unwrap()
        .0;
    let subscription = h.snapshot().subscriptions[0].active.unwrap();
    let out = h.send(Input::SurfaceRemoved(SurfaceId(1)));
    assert!(!h.effects.cancelled.contains(&committed));
    assert!(h.effects.cancelled.contains(&transient));
    assert!(out.contains(&Output::Unsubscribe { subscription }));
    let start = h.trace.len();
    h.advance_ms(1000);
    assert!(
        h.trace[start..]
            .iter()
            .any(|e| matches!(e.input,Input::EffectDone{request,..} if request==committed))
    );
    assert!(h.trace[start..].iter().all(|e| {
        e.outputs
            .iter()
            .all(|o| !matches!(o,Output::CallLogic{instance,..} if *instance==owner))
    }));
    checked(h);
}
#[test]
fn replay_from_initial_checkpoint() {
    let mut core = Core::new(CoreConfig::default());
    let mut recorder = Recorder::new(&core);
    for n in 0..500 {
        recorder.handle(
            &mut core,
            Timestamp(n),
            Input::Fact {
                name: "counter".into(),
                value: Some(FactValue::Number(n as f64)),
            },
        );
    }
    let dump = recorder.dump();
    assert_eq!(dump.inputs.len(), 500);
    assert_eq!(
        Core::restore(dump.checkpoint.clone())
            .unwrap()
            .snapshot()
            .now,
        Timestamp(0)
    );
    let json = dump.to_json().unwrap();
    assert_eq!(
        ReplayDump::from_json(&json)
            .unwrap()
            .replay()
            .unwrap()
            .core
            .snapshot(),
        core.snapshot()
    );
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["format_version"] = serde_json::json!(DUMP_VERSION + 1);
    assert!(matches!(
        ReplayDump::from_json(&value.to_string()),
        Err(ReplayError::Version(_))
    ));
}
#[test]
fn fault_backoff() {
    let mut logic = FakeLogic::default();
    logic.register("controls/main", None, |id, _| {
        if id.number == 1 {
            LogicReaction::Fault("trap".into())
        } else {
            LogicReaction::Update(LogicUpdate::default())
        }
    });
    let mut h = setup(
        fixtures::controls_with_panel(),
        logic,
        CoreConfig::default(),
        &[info(1), info(2)],
    );
    let other = h
        .snapshot()
        .instances
        .iter()
        .find(|i| i.surface == SurfaceId(2))
        .unwrap()
        .clone();
    for deadline in [1000, 3000, 7000] {
        let snap = h.snapshot();
        let fault = snap
            .instances
            .iter()
            .find(|i| i.surface == SurfaceId(1))
            .unwrap();
        assert_eq!(fault.status, InstanceStatus::Faulted);
        assert_eq!(fault.retry_at, Some(Timestamp(deadline)));
        let generation = fault.id.generation;
        h.advance_ms(deadline - h.now().0 - 1);
        assert_eq!(
            h.snapshot()
                .instances
                .iter()
                .find(|i| i.surface == SurfaceId(1))
                .unwrap()
                .id
                .generation,
            generation
        );
        h.advance_ms(1);
        assert!(
            h.snapshot()
                .instances
                .iter()
                .find(|i| i.surface == SurfaceId(1))
                .unwrap()
                .id
                .generation
                > generation
        );
        let snap = h.snapshot();
        let stable = snap
            .instances
            .iter()
            .find(|i| i.surface == SurfaceId(2))
            .unwrap();
        assert_eq!(stable.id, other.id);
        assert_eq!(stable.state, other.state);
        assert_eq!(stable.status, InstanceStatus::Visible);
    }
    checked(h);
}

#[test]
fn surfaces_are_independent() {
    let mut c = alternate_catalog();
    c.profiles[0].when = Some(Condition::Eq {
        name: "surface.kind".into(),
        value: FactValue::Text("touchbar".into()),
    });
    c.profiles[1].for_target = None;
    c.profiles[1].default = true;
    c.profiles[1].priority = 1;
    c.profiles[1].when = Some(Condition::Eq {
        name: "surface.kind".into(),
        value: FactValue::Text("window".into()),
    });
    let mut first = info(1);
    first.kind = SurfaceKind::Touchbar;
    first.name = "touchbar".parse().unwrap();
    let mut second = info(2);
    second.kind = SurfaceKind::Window;
    second.name = "window:test".parse().unwrap();
    let mut h = setup(
        c,
        FakeLogic::default(),
        CoreConfig::default(),
        &[first, second],
    );
    assert_eq!(
        h.snapshot().surfaces[0]
            .profile
            .active
            .as_ref()
            .unwrap()
            .to_string(),
        "controls/default"
    );
    assert_eq!(
        h.snapshot().surfaces[1]
            .profile
            .active
            .as_ref()
            .unwrap()
            .to_string(),
        "controls/editor"
    );
    h.touch(
        SurfaceId(2),
        1,
        TouchPhase::Down,
        h.hit(SurfaceId(2), "volume"),
    );
    let capture = h.snapshot().captures[0].clone();
    h.touch(
        SurfaceId(1),
        1,
        TouchPhase::Down,
        h.hit(SurfaceId(1), "microphone"),
    );
    h.advance_ms(100);
    assert!(h.snapshot().surfaces[0].presentation.is_some());
    assert!(h.snapshot().surfaces[1].presentation.is_none());
    assert!(h.snapshot().captures.contains(&capture));
    fn_key(&mut h, 1, true);
    assert!(h.snapshot().surfaces[1].layers.is_empty());
    assert!(h.snapshot().surfaces[1].presentation.is_none());
    assert!(h.snapshot().captures.contains(&capture));
    checked(h);
}
#[test]
fn same_set_on_two_surfaces() {
    let mut first = info(1);
    first.caps.modes = true;
    let mut second = info(2);
    second.caps.modes = true;
    let mut h = setup(
        fixtures::controls_with_panel(),
        FakeLogic::default(),
        CoreConfig::default(),
        &[first, second],
    );
    for surface in [SurfaceId(1), SurfaceId(2)] {
        h.send(Input::Mode {
            surface,
            mode: Mode::Display,
        });
    }
    let before = h.snapshot();
    let first = before
        .instances
        .iter()
        .find(|i| i.surface == SurfaceId(1))
        .unwrap()
        .id;
    let other = before
        .instances
        .iter()
        .find(|i| i.surface == SurfaceId(2))
        .unwrap()
        .clone();
    assert_ne!(first, other.id);
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Native,
    });
    let snap = h.snapshot();
    assert_eq!(
        snap.instances
            .iter()
            .find(|i| i.id == first)
            .unwrap()
            .status,
        InstanceStatus::Hidden
    );
    assert_eq!(
        snap.instances
            .iter()
            .find(|i| i.id == other.id)
            .unwrap()
            .status,
        InstanceStatus::Visible
    );
    h.logic.register("controls/main", None, move |id, _| {
        if id == first {
            LogicReaction::Fault("trap".into())
        } else {
            LogicReaction::Update(LogicUpdate::default())
        }
    });
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Display,
    });
    assert_eq!(
        h.snapshot()
            .instances
            .iter()
            .find(|i| i.id == first)
            .unwrap()
            .status,
        InstanceStatus::Faulted
    );
    let snap = h.snapshot();
    let stable = snap.instances.iter().find(|i| i.id == other.id).unwrap();
    assert_eq!(stable.status, other.status);
    assert_eq!(stable.state, other.state);
    checked(h);
}
#[test]
fn fn_layer_only_on_source_surface() {
    for all in [false, true] {
        let mut c = fixtures::controls_with_panel();
        if all {
            c.bindings[0].action.surfaces = LayerSurfaces::All;
        }
        let mut h = setup(
            c,
            FakeLogic::default(),
            CoreConfig::default(),
            &[info(1), info(2)],
        );
        fn_key(&mut h, 1, true);
        let s = h.snapshot();
        assert_eq!(s.surfaces[0].layers.len(), 1);
        assert_eq!(s.surfaces[1].layers.len(), usize::from(all));
        checked(h);
    }
}
#[test]
fn thresholds_follow_density() {
    let c = catalog()
        .plugin("controls")
        .set("tracks", |s| {
            s.scrubber("records");
        })
        .profile("default", |p| {
            p.default().slot("main", &["controls/tracks"]);
        })
        .build();
    for millimetres in [0.9_f32, 1.1] {
        let mut outcomes = vec![];
        for density in [2.0, 10.0] {
            let mut logic = FakeLogic::default();
            logic.register("controls/tracks", None, |_, call| {
                let mut update = LogicUpdate::default();
                if matches!(call, LogicCall::Init { .. }) {
                    update.state = vec![(
                        "records_items".into(),
                        StateValue::Records(
                            (0..5)
                                .map(|n| {
                                    BTreeMap::from([
                                        ("key".into(), Scalar::Text(n.to_string())),
                                        ("text".into(), Scalar::Text(format!("track{n}"))),
                                    ])
                                })
                                .collect(),
                        ),
                    )];
                }
                LogicReaction::Update(update)
            });
            let mut surface = info(1);
            surface.px_per_mm = density;
            let mut h = setup(c.clone(), logic, CoreConfig::default(), &[surface]);
            h.touch(
                SurfaceId(1),
                1,
                TouchPhase::Down,
                Point { x: 40.0, y: 25.0 },
            );
            h.touch(
                SurfaceId(1),
                1,
                TouchPhase::Move,
                Point {
                    x: 40.0 - millimetres * density,
                    y: 25.0,
                },
            );
            let panned = h.snapshot().surfaces[0].scene.items[0].interactions[0]
                .scrubber
                .offset_px
                > 0.0;
            outcomes.push(panned);
            checked(h);
        }
        assert_eq!(outcomes, vec![millimetres >= 1.0; 2]);
    }
}
#[test]
fn surface_removed_mid_touch() {
    let mut h = ready();
    fn_key(&mut h, 1, true);
    h.touch(SurfaceId(1), 1, TouchPhase::Down, h.hit(SurfaceId(1), "f1"));
    let capture = h.snapshot().captures[0].clone();
    assert_eq!(h.snapshot().key_leases.len(), 1);
    let out = h.send(Input::SurfaceRemoved(SurfaceId(1)));
    assert!(out.contains(&Output::CaptureFinished {
        capture,
        completion: CaptureCompletion::Cancel(CancelReason::SurfaceRemoved)
    }));
    assert!(out.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    assert!(h.snapshot().surfaces.is_empty());
    assert!(h.snapshot().instances.is_empty());
    assert!(h.snapshot().key_leases.is_empty());
    checked(h);
}
#[test]
fn key_without_inject_goes_to_desktop() {
    let mut surface = info(1);
    surface.caps.inject_keys = false;
    let mut h = setup(
        fixtures::controls_with_panel(),
        FakeLogic::default(),
        CoreConfig::default(),
        &[surface],
    );
    h.send(Input::DesktopCapabilities(DesktopCapabilities {
        key: true,
    }));
    fn_key(&mut h, 1, true);
    let start = h.trace.len();
    h.tap(SurfaceId(1), "f1");
    let out: Vec<_> = h.trace[start..]
        .iter()
        .flat_map(|e| e.outputs.clone())
        .collect();
    assert!(out.contains(&Output::DesktopKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Pressed
    }));
    assert!(out.contains(&Output::DesktopKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    assert!(!out.iter().any(|o| matches!(o, Output::SendKey { .. })));
    checked(h);
}
