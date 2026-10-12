use super::*;

fn assert_state(actual: &Core, expected: &Core) {
    fn difference(a: &serde_json::Value, b: &serde_json::Value, path: String) -> Option<String> {
        if a == b {
            return None;
        }
        match (a, b) {
            (serde_json::Value::Object(a), serde_json::Value::Object(b)) if a.len() == b.len() => {
                a.iter().find_map(|(key, v)| {
                    difference(
                        v,
                        b.get(key).unwrap_or(&serde_json::Value::Null),
                        format!("{path}/{key}"),
                    )
                })
            }
            (serde_json::Value::Array(a), serde_json::Value::Array(b)) if a.len() == b.len() => a
                .iter()
                .zip(b)
                .enumerate()
                .find_map(|(n, (a, b))| difference(a, b, format!("{path}/{n}"))),
            _ => Some(format!("{path}: {a} != {b}")),
        }
    }
    let a = serde_json::to_value(actual.checkpoint()).unwrap();
    let b = serde_json::to_value(expected.checkpoint()).unwrap();
    assert!(a == b, "{}", difference(&a, &b, String::new()).unwrap());
}

fn movement(surface: u64, contact: u32, phase: TouchPhase, x: f32) -> Input {
    Input::Touch {
        surface: SurfaceId(surface),
        contact: ContactId::new(contact).unwrap(),
        phase,
        x,
        y: 20.0,
    }
}
fn setup(kind: WidgetKind, logic: bool, slots: usize) -> Core {
    let mut cat = Catalog::default();
    cat.plugins.push(PluginSpec {
        id: "p".parse().unwrap(),
        revision: "1".into(),
        enabled: true,
        has_logic: true,
        requires: vec![],
    });
    let mut profile = ProfileSpec::default_for("p/default".parse().unwrap());
    profile.default = true;
    for n in 0..slots {
        let mut set = SetSpec::default_for(format!("p/set{n}").parse().unwrap());
        set.logic = logic.then(|| "p".parse().unwrap());
        let mut control = WidgetSpec::new("control".parse().unwrap(), kind.clone());
        if logic {
            for k in [
                ActionKind::Pressed,
                ActionKind::Released,
                ActionKind::Activated,
                ActionKind::ValueChanged,
                ActionKind::Selected,
                ActionKind::Cancelled,
            ] {
                control.handlers.insert(k, format!("{k:?}"));
            }
        }
        set.widgets.push(control);
        for label in 0..8 {
            set.widgets.push(WidgetSpec::new(
                format!("label{label}").parse().unwrap(),
                WidgetKind::Noninteractive,
            ));
        }
        profile.slots.push(SlotSpec {
            id: format!("slot{n}").parse().unwrap(),
            candidates: vec![set.reference.clone()],
        });
        cat.sets.push(set);
    }
    cat.profiles.push(profile.clone());
    profile.reference = "p/other".parse().unwrap();
    profile.default = false;
    profile.for_target = Some(ProfileTarget::App("other".into()));
    cat.profiles.push(profile);
    assert!(cat.validate().is_empty());
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::Catalog(cat));
    for n in 1..=2 {
        c.handle(
            Timestamp(0),
            Input::SurfaceAdded(SurfaceInfo {
                id: SurfaceId(n),
                name: format!("headless:{n}").parse().unwrap(),
                kind: SurfaceKind::Headless,
                width_px: 1000,
                height_px: 100,
                px_per_mm: 5.0,
                caps: SurfaceCaps {
                    modes: true,
                    ..SurfaceCaps::default()
                },
            }),
        );
        c.handle(
            Timestamp(0),
            Input::Mode {
                surface: SurfaceId(n),
                mode: Mode::Display,
            },
        );
    }
    if logic {
        let ids: Vec<_> = c.instances.values().map(|i| i.id).collect();
        for id in ids {
            c.handle(
                Timestamp(0),
                Input::LogicDescribed {
                    instance: id,
                    persisted: vec![],
                    description: LogicDescription {
                        state: vec![],
                        actions: [
                            ActionKind::Pressed,
                            ActionKind::Released,
                            ActionKind::Activated,
                            ActionKind::ValueChanged,
                            ActionKind::Selected,
                            ActionKind::Cancelled,
                        ]
                        .into_iter()
                        .map(|k| {
                            (
                                format!("{k:?}"),
                                ActionDescription {
                                    kinds: vec![k],
                                    value: (k == ActionKind::ValueChanged)
                                        .then_some(ScalarKind::Number),
                                },
                            )
                        })
                        .collect(),
                    },
                },
            );
        }
        while let Some((instance, call)) = c
            .instances
            .values()
            .find_map(|i| i.snapshot().in_flight.map(|call| (i.id, call)))
        {
            c.handle(
                Timestamp(0),
                Input::LogicUpdate {
                    instance,
                    call,
                    update: LogicUpdate::default(),
                },
            );
        }
    }
    for n in 1..=2 {
        let id = c.surfaces[&SurfaceId(n)].scene.items[0].instance;
        let rect = Rect {
            x: 10.0,
            y: 10.0,
            width: 100.0,
            height: 20.0,
        };
        c.handle(
            Timestamp(1),
            Input::Layout {
                surface: SurfaceId(n),
                generation: c.surfaces[&SurfaceId(n)].generation,
                hit_map: vec![HitTarget {
                    instance: id,
                    widget: "control".parse().unwrap(),
                    plane: ScenePlane::Profile,
                    rect,
                    record_key: None,
                }],
                scrubbers: if kind == WidgetKind::Scrubber {
                    vec![ScrubberLayout {
                        instance: id,
                        widget: "control".parse().unwrap(),
                        viewport: rect,
                        content_width: 300.0,
                        items: vec![ScrubberItemGeometry {
                            record_key: "a".into(),
                            rect: Rect {
                                x: 0.0,
                                y: 0.0,
                                width: 100.0,
                                height: 20.0,
                            },
                        }],
                    }]
                } else {
                    vec![]
                },
                slots: vec![],
                representations: vec![],
                presentation: None,
            },
        );
        c.handle(Timestamp(1), movement(n, 1, TouchPhase::Down, 90.0));
    }
    assert_eq!(c.snapshot().captures.len(), 2);
    c
}

#[test]
#[ignore = "manual CPU benchmark; no timing threshold in the test suite"]
fn benchmark_touch_moves() {
    for full in [true, false] {
        let mut c = setup(
            WidgetKind::Slider {
                minimum: 0.0,
                maximum: 1.0,
            },
            false,
            20,
        );
        let start = std::time::Instant::now();
        for n in 0..10_000 {
            let at = Timestamp(2 + n);
            let input = movement(1, 1, TouchPhase::Move, (n % 100) as f32 + 10.0);
            std::hint::black_box(if full {
                c.handle_full(at, input)
            } else {
                c.handle(at, input)
            });
        }
        eprintln!(
            "10,000 moves; 2 surfaces, 40 instances, 360 widgets; full={full}: {:?}",
            start.elapsed()
        );
    }
}

#[test]
fn moves_preserve_subscription_retries_before_at_and_after_deadline() {
    for background in [false, true] {
        for surface in [1, 2, 99] {
            for contact in [1, 2] {
                for at in [1002, 1003, 1004] {
                    let mut fast = setup(WidgetKind::Button, true, 3);
                    let owner = fast.surfaces[&SurfaceId(1)].scene.items[0].instance;
                    let call = fast.instances[&owner.number].snapshot().in_flight.unwrap();
                    let outputs = fast.handle(
                        Timestamp(2),
                        Input::LogicUpdate {
                            instance: owner,
                            call,
                            update: LogicUpdate {
                                subscriptions: Some(vec![SubscriptionSpec {
                                    tag: "audio".into(),
                                    kind: SubscriptionKind::AudioWatch {
                                        target: AudioTarget::DefaultSink,
                                    },
                                    background,
                                }]),
                                ..LogicUpdate::default()
                            },
                        },
                    );
                    let subscription = outputs
                        .iter()
                        .find_map(|o| match o {
                            Output::Subscribe { subscription, .. } => Some(*subscription),
                            _ => None,
                        })
                        .unwrap();
                    fast.handle(
                        Timestamp(3),
                        Input::SubscriptionLost {
                            subscription,
                            error: EffectError::Unavailable,
                        },
                    );
                    let mut reference = Core::restore(fast.checkpoint()).unwrap();
                    let input = movement(surface, contact, TouchPhase::Move, 50.0);
                    let expected = reference.handle_full(Timestamp(at), input.clone());
                    assert_eq!(
                        expected
                            .iter()
                            .filter(|o| matches!(o, Output::Subscribe { .. }))
                            .count(),
                        usize::from(at >= 1003)
                    );
                    assert_eq!(
                        fast.handle(Timestamp(at), input),
                        expected,
                        "background={background}, surface={surface}, contact={contact}, at={at}"
                    );
                    assert_state(&fast, &reference);
                }
            }
        }
    }
}

#[test]
fn moves_preserve_outputs_and_complete_state_of_full_synchronization() {
    for kind in [
        WidgetKind::Button,
        WidgetKind::Slider {
            minimum: 0.0,
            maximum: 1.0,
        },
        WidgetKind::Scrubber,
    ] {
        for logic in [false, true] {
            for pending in [false, true] {
                let mut fast = setup(kind.clone(), logic, 3);
                let mut reference = fast.clone();
                {
                    let mut step = |at, input: Input| {
                        assert_eq!(
                            fast.handle(Timestamp(at), input.clone()),
                            reference.handle_full(Timestamp(at), input.clone()),
                            "{kind:?}, logic={logic}, input={input:?}"
                        );
                        assert_state(&fast, &reference);
                    };
                    for n in 2..40 {
                        step(
                            n,
                            movement(1 + n % 2, 1, TouchPhase::Move, (n % 100) as f32),
                        );
                        if n == 10 && pending {
                            step(
                                n,
                                Input::Fact {
                                    name: "app.id".into(),
                                    value: Some(FactValue::Text("other".into())),
                                },
                            );
                        }
                    }
                    // Both surfaces may have a frame in progress and a serialized call blocked.
                }
                for at in 40..48 {
                    let ids: Vec<_> = fast
                        .instances
                        .values()
                        .filter_map(|i| i.snapshot().in_flight.map(|call| (i.id, call)))
                        .collect();
                    for (instance, call) in ids {
                        let input = Input::LogicUpdate {
                            instance,
                            call,
                            update: LogicUpdate {
                                requests: vec![EffectRequest {
                                    tag: "write".into(),
                                    effect: Effect::MediaNext,
                                    lifetime: None,
                                    timeout_ms: None,
                                    coalesce: Some("move".into()),
                                }],
                                ..LogicUpdate::default()
                            },
                        };
                        assert_eq!(
                            fast.handle(Timestamp(at), input.clone()),
                            reference.handle_full(Timestamp(at), input)
                        );
                    }
                    for surface in 1..=2 {
                        let input = Input::FrameDone {
                            surface: SurfaceId(surface),
                            generation: fast.surfaces[&SurfaceId(surface)].generation,
                        };
                        assert_eq!(
                            fast.handle(Timestamp(at), input.clone()),
                            reference.handle_full(Timestamp(at), input)
                        );
                        let input = movement(surface, 1, TouchPhase::Move, 90.0 - at as f32);
                        assert_eq!(
                            fast.handle(Timestamp(at), input.clone()),
                            reference.handle_full(Timestamp(at), input)
                        );
                    }
                    assert_state(&fast, &reference);
                    fast = Core::restore(fast.checkpoint()).unwrap();
                }
                for input in [
                    movement(1, 1, TouchPhase::Move, f32::NAN),
                    movement(2, 2, TouchPhase::Move, 50.0),
                    movement(99, 1, TouchPhase::Move, 50.0),
                    movement(1, 1, TouchPhase::Up, 30.0),
                    Input::Mode {
                        surface: SurfaceId(2),
                        mode: Mode::Native,
                    },
                    movement(2, 1, TouchPhase::Move, 60.0),
                ] {
                    assert_eq!(
                        fast.handle(Timestamp(50), input.clone()),
                        reference.handle_full(Timestamp(50), input)
                    );
                    assert_state(&fast, &reference);
                }
                let input = movement(1, 1, TouchPhase::Move, 50.0);
                assert_eq!(
                    fast.handle(Timestamp(49), input.clone()),
                    reference.handle_full(Timestamp(49), input)
                );
            }
        }
    }
}
