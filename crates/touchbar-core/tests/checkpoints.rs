use proptest::prelude::*;
use touchbar_core::*;
use touchbar_model::*;

fn info() -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(1),
        name: "headless:one".parse().unwrap(),
        kind: SurfaceKind::Headless,
        width_px: 500,
        height_px: 50,
        px_per_mm: 5.0,
        caps: SurfaceCaps {
            watch_keys: true,
            inject_keys: true,
            modes: true,
            ..SurfaceCaps::default()
        },
    }
}
fn catalog() -> Catalog {
    let mut set = SetSpec::default_for("p/main".parse().unwrap());
    set.logic = Some("p".parse().unwrap());
    let mut profile = ProfileSpec::default_for("p/default".parse().unwrap());
    profile.default = true;
    profile.slots.push(SlotSpec {
        id: "main".parse().unwrap(),
        candidates: vec![set.reference.clone()],
    });
    Catalog {
        plugins: vec![PluginSpec {
            id: "p".parse().unwrap(),
            revision: "1".into(),
            enabled: true,
            has_logic: true,
            requires: vec![],
        }],
        sets: vec![set],
        profiles: vec![profile],
        bindings: vec![Binding {
            id: "fn".into(),
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
fn roundtrip(c: &Core) -> Core {
    let json = serde_json::to_vec(&c.checkpoint()).unwrap();
    Core::restore(serde_json::from_slice(&json).unwrap()).unwrap()
}
fn description() -> LogicDescription {
    LogicDescription {
        state: vec![StateField {
            name: "n".into(),
            kind: FieldKind::Scalar(ScalarKind::Number),
            persist: true,
        }],
        ..LogicDescription::default()
    }
}
fn setup() -> Core {
    let mut c = Core::new(CoreConfig {
        effect_slots: 1,
        persist_delay_ms: 73,
        ..CoreConfig::default()
    });
    c.handle(Timestamp(0), Input::Catalog(catalog()));
    c.handle(Timestamp(0), Input::SurfaceAdded(info()));
    c.handle(
        Timestamp(0),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Display,
        },
    );
    let id = c.snapshot().instances[0].id;
    c.handle(
        Timestamp(0),
        Input::LogicDescribed {
            instance: id,
            description: description(),
            persisted: vec![],
        },
    );
    c
}
fn request(tag: &str) -> EffectRequest {
    EffectRequest {
        tag: tag.into(),
        effect: Effect::MediaNext,
        lifetime: Some(Lifetime::Committed),
        coalesce: None,
        timeout_ms: Some(500),
    }
}
#[test]
fn restores_embedded_config_pending_calls_orphan_effects_and_persistence_timer() {
    let mut c = setup();
    let i = c.snapshot().instances[0].clone();
    let out = c.handle(
        Timestamp(1),
        Input::LogicUpdate {
            instance: i.id,
            call: i.in_flight.unwrap(),
            update: LogicUpdate {
                state: vec![("n".into(), StateValue::Scalar(Scalar::Number(7.0)))],
                requests: vec![request("a"), request("b")],
                ..LogicUpdate::default()
            },
        },
    );
    let timer = out
        .iter()
        .find_map(|o| {
            if let Output::SetTimer { id, at } = o {
                assert_eq!(*at, Timestamp(74));
                Some(*id)
            } else {
                None
            }
        })
        .unwrap();
    let mut restored = roundtrip(&c);
    assert_eq!(restored.snapshot(), c.snapshot());
    assert_eq!(restored.checkpoint().config().persist_delay_ms, 73);
    let id = c.snapshot().instances[0].id;
    let call = c.snapshot().instances[0].in_flight.unwrap();
    let inputs = vec![
        (
            Timestamp(2),
            Input::LogicUpdate {
                instance: id,
                call,
                update: LogicUpdate::default(),
            },
        ),
        (Timestamp(3), Input::SurfaceRemoved(SurfaceId(1))),
        (Timestamp(74), Input::Timer { id: timer }),
        (
            Timestamp(75),
            Input::EffectDone {
                request: out
                    .iter()
                    .find_map(|o| {
                        if let Output::StartEffect { request, .. } = o {
                            Some(*request)
                        } else {
                            None
                        }
                    })
                    .unwrap(),
                result: EffectOutcome::Ok(EffectResult::None),
            },
        ),
        (Timestamp(76), Input::SurfaceAdded(info())),
    ];
    for (at, input) in inputs {
        assert_eq!(restored.handle(at, input.clone()), c.handle(at, input));
        assert_eq!(restored.snapshot(), c.snapshot());
    }
}
#[test]
fn unknown_version_is_an_explicit_restore_error() {
    let mut state = Core::new(CoreConfig::default()).checkpoint();
    state.format_version = u32::MAX;
    assert!(matches!(
        Core::restore(state),
        Err(RestoreError::Version {
            found: u32::MAX,
            ..
        })
    ));
}
#[test]
fn initial_checkpoint_continues_identically() {
    let mut c = Core::new(CoreConfig::default());
    let mut restored = roundtrip(&c);
    for input in [
        Input::Catalog(catalog()),
        Input::SurfaceAdded(info()),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Display,
        },
    ] {
        assert_eq!(
            c.handle(Timestamp(0), input.clone()),
            restored.handle(Timestamp(0), input)
        );
    }
}
#[test]
fn snapshot_exposes_queued_calls_requests_subscriptions_and_layer_ownership() {
    let mut c = setup();
    let i = c.snapshot().instances[0].clone();
    c.handle(
        Timestamp(1),
        Input::LogicUpdate {
            instance: i.id,
            call: i.in_flight.unwrap(),
            update: LogicUpdate {
                requests: vec![request("running"), request("waiting")],
                subscriptions: Some(vec![SubscriptionSpec {
                    tag: "clock".into(),
                    kind: SubscriptionKind::TimerInterval { ms: 100 },
                    background: false,
                }]),
                ..LogicUpdate::default()
            },
        },
    );
    c.handle(
        Timestamp(2),
        Input::SubscriptionData {
            subscription: c.snapshot().subscriptions[0].active.unwrap(),
            data: SubscriptionData::Tick,
        },
    );
    c.handle(
        Timestamp(3),
        Input::Key {
            source: KeySource::Surface(SurfaceId(1)),
            code: WatchKey::Fn,
            phase: KeyPhase::Pressed,
        },
    );
    let snap = c.snapshot();
    assert_eq!(snap.requests.len(), 2);
    assert!(snap.requests[0].running);
    assert!(!snap.requests[1].running);
    assert_eq!(snap.instances[0].requests, snap.requests);
    assert_eq!(snap.instances[0].subscriptions, snap.subscriptions);
    assert_eq!(
        snap.instances[0].queue,
        vec![LogicCall::Handle(LogicEvent::SubscriptionData {
            tag: "clock".into(),
            data: SubscriptionData::Tick
        })]
    );
    assert!(snap.instances[0].call.is_some());
    assert_eq!(
        snap.surfaces[0].layers[0].held,
        vec![(KeySource::Surface(SurfaceId(1)), "fn".into())]
    );
    let json = serde_json::to_vec(&snap).unwrap();
    assert_eq!(serde_json::from_slice::<CoreSnapshot>(&json).unwrap(), snap);
    c.handle(Timestamp(4), Input::SurfaceRemoved(SurfaceId(1)));
    assert!(c.snapshot().instances.is_empty());
    assert_eq!(c.snapshot().requests.len(), 2);
}

// The host response is derived solely from the input command, so both cores
// receive identical completions. Some calls are deliberately left in flight.
fn stimulus(c: &Core, code: u8, at: u64) -> Input {
    let s = c.snapshot();
    match code % 12 {
        0 => Input::Key {
            source: KeySource::Surface(SurfaceId(1)),
            code: WatchKey::Fn,
            phase: KeyPhase::Pressed,
        },
        1 => Input::Key {
            source: KeySource::Surface(SurfaceId(1)),
            code: WatchKey::Fn,
            phase: KeyPhase::Released,
        },
        2 => Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Native,
        },
        3 => Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Display,
        },
        4 => Input::Fact {
            name: FactName("app.id".into()),
            value: Some(FactValue::Text(format!("app-{code}"))),
        },
        5 => Input::SurfaceRemoved(SurfaceId(1)),
        6 => Input::SurfaceAdded(info()),
        7 => {
            let mut cat = catalog();
            cat.plugins[0].revision = format!("{code}");
            Input::Catalog(cat)
        }
        8 | 9 => {
            if let Some(i) = s.instances.iter().find(|i| i.in_flight.is_some()) {
                Input::LogicUpdate {
                    instance: i.id,
                    call: i.in_flight.unwrap(),
                    update: LogicUpdate {
                        state: vec![(
                            "n".into(),
                            StateValue::Scalar(Scalar::Number(f64::from(code))),
                        )],
                        requests: vec![request("job")],
                        ..LogicUpdate::default()
                    },
                }
            } else if let Some(i) = s
                .instances
                .iter()
                .find(|i| i.status == InstanceStatus::Loading)
            {
                Input::LogicDescribed {
                    instance: i.id,
                    description: description(),
                    persisted: vec![],
                }
            } else {
                Input::Timer { id: TimerId(at) }
            }
        }
        10 => s
            .requests
            .first()
            .map(|r| Input::EffectDone {
                request: r.id,
                result: EffectOutcome::Ok(EffectResult::None),
            })
            .unwrap_or(Input::Timer { id: TimerId(at) }),
        _ => Input::Timer {
            id: TimerId(at % 30 + 1),
        },
    }
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]
    #[test]
    fn random_checkpoint_preserves_exact_continuation(steps in prop::collection::vec((any::<u8>(),0u16..200),1..100), split in any::<usize>()) {
        let split=split % (steps.len()+1);let mut c=setup();let mut now=0u64;
        for (code,delta) in &steps[..split] {now+=u64::from(*delta);let input=stimulus(&c,*code,now);c.handle(Timestamp(now),input);}
        let mut restored=roundtrip(&c);
        for (code,delta) in &steps[split..] {
            now+=u64::from(*delta);let input=stimulus(&c,*code,now);
            prop_assert_eq!(restored.handle(Timestamp(now),input.clone()),c.handle(Timestamp(now),input));
            prop_assert_eq!(restored.snapshot(),c.snapshot());
            prop_assert_eq!(serde_json::to_value(restored.checkpoint()).unwrap(),serde_json::to_value(c.checkpoint()).unwrap());
        }
    }
}

#[test]
fn checkpoint_restores_capture_key_leases_panel_and_frame_acknowledgement() {
    let mut cat = catalog();
    cat.sets[0].logic = None;
    let mut opener = WidgetSpec::new("open".parse().unwrap(), WidgetKind::Button);
    opener.expanded_panel = Some("panel".parse().unwrap());
    cat.sets[0].widgets = vec![opener];
    cat.sets[0].panels = vec![PanelSpec {
        id: "panel".parse().unwrap(),
        widgets: vec![WidgetSpec::new(
            "key".parse().unwrap(),
            WidgetKind::Key { key: SystemKey::F1 },
        )],
        dismiss_on_selection: false,
        principal_item: None,
        dismissal: DismissalPolicy {
            release: false,
            timeout_ms: Some(500),
            ..DismissalPolicy::default()
        },
    }];
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::Catalog(cat));
    c.handle(Timestamp(0), Input::SurfaceAdded(info()));
    c.handle(
        Timestamp(0),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Display,
        },
    );
    let s = c.snapshot().surfaces[0].clone();
    let owner = s.scene.items[0].instance;
    let rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 50.0,
    };
    c.handle(
        Timestamp(1),
        Input::Layout {
            surface: SurfaceId(1),
            generation: s.generation,
            hit_map: vec![HitTarget {
                instance: owner,
                widget: "open".parse().unwrap(),
                rect,
                plane: ScenePlane::Profile,
                record_key: None,
            }],
            representations: vec![],
            scrubbers: vec![],
            slots: vec![SlotBounds {
                instance: owner,
                rect,
            }],
            presentation: None,
        },
    );
    let touch = |phase| Input::Touch {
        surface: SurfaceId(1),
        contact: ContactId::new(1).unwrap(),
        phase,
        x: 10.0,
        y: 20.0,
    };
    c.handle(Timestamp(2), touch(TouchPhase::Down));
    let mut restored = roundtrip(&c);
    assert_eq!(restored.snapshot(), c.snapshot());
    assert_eq!(
        restored.handle(Timestamp(3), touch(TouchPhase::Up)),
        c.handle(Timestamp(3), touch(TouchPhase::Up))
    );
    assert!(c.snapshot().surfaces[0].presentation.is_some());
    // An old frame is still outstanding. Its acknowledgement must allow both
    // restored and original cores to render the newly opened panel identically.
    let ack = Input::FrameDone {
        surface: SurfaceId(1),
        generation: s.generation,
    };
    assert_eq!(
        restored.handle(Timestamp(4), ack.clone()),
        c.handle(Timestamp(4), ack)
    );
    let s = c.snapshot().surfaces[0].clone();
    let panel = s.presentation.unwrap();
    let key_hit = HitTarget {
        instance: owner,
        widget: "key".parse().unwrap(),
        rect,
        plane: ScenePlane::Presentation,
        record_key: None,
    };
    let layout = Input::Layout {
        surface: SurfaceId(1),
        generation: s.generation,
        hit_map: vec![key_hit],
        representations: vec![],
        scrubbers: vec![],
        slots: vec![SlotBounds {
            instance: owner,
            rect,
        }],
        presentation: Some(PresentationLayout {
            id: panel.id,
            bounds: rect,
        }),
    };
    assert_eq!(
        restored.handle(Timestamp(5), layout.clone()),
        c.handle(Timestamp(5), layout)
    );
    assert_eq!(
        restored.handle(Timestamp(6), touch(TouchPhase::Down)),
        c.handle(Timestamp(6), touch(TouchPhase::Down))
    );
    let mut restored = roundtrip(&c);
    assert_eq!(restored.snapshot(), c.snapshot());
    let out = c.handle(
        Timestamp(7),
        Input::Revoked {
            surface: SurfaceId(1),
        },
    );
    assert_eq!(
        restored.handle(
            Timestamp(7),
            Input::Revoked {
                surface: SurfaceId(1)
            }
        ),
        out
    );
    assert!(out.iter().any(|o| matches!(
        o,
        Output::SendKey {
            key: SystemKey::F1,
            phase: KeyPhase::Released,
            ..
        }
    )));
}
#[test]
fn checkpoint_restores_subscription_retry_deadline_and_generation() {
    let mut c = setup();
    let i = c.snapshot().instances[0].clone();
    let out = c.handle(
        Timestamp(1),
        Input::LogicUpdate {
            instance: i.id,
            call: i.in_flight.unwrap(),
            update: LogicUpdate {
                subscriptions: Some(vec![SubscriptionSpec {
                    tag: "clock".into(),
                    kind: SubscriptionKind::TimerInterval { ms: 100 },
                    background: true,
                }]),
                ..LogicUpdate::default()
            },
        },
    );
    let subscription = out
        .iter()
        .find_map(|o| {
            if let Output::Subscribe { subscription, .. } = o {
                Some(*subscription)
            } else {
                None
            }
        })
        .unwrap();
    let out = c.handle(
        Timestamp(2),
        Input::SubscriptionLost {
            subscription,
            error: EffectError::Unavailable,
        },
    );
    let (timer, at) = out
        .iter()
        .find_map(|o| {
            if let Output::SetTimer { id, at } = o {
                Some((*id, *at))
            } else {
                None
            }
        })
        .unwrap();
    let mut restored = roundtrip(&c);
    let out = c.handle(at, Input::Timer { id: timer });
    assert_eq!(restored.handle(at, Input::Timer { id: timer }), out);
    assert!(
        out.iter()
            .any(|o| matches!(o,Output::Subscribe {subscription:new,..} if *new!=subscription))
    );
    assert_eq!(
        restored.handle(
            at,
            Input::SubscriptionData {
                subscription,
                data: SubscriptionData::Tick
            }
        ),
        c.handle(
            at,
            Input::SubscriptionData {
                subscription,
                data: SubscriptionData::Tick
            }
        )
    );
    assert_eq!(restored.snapshot(), c.snapshot());
}
