use touchbar_core::*;
use touchbar_model::*;
fn catalog() -> Catalog {
    let mut set = SetSpec::default_for("p/main".parse().unwrap());
    set.logic = Some("p".parse().unwrap());
    let mut button = WidgetSpec::new("button".parse().unwrap(), WidgetKind::Button);
    button.handlers.insert(ActionKind::Pressed, "press".into());
    set.widgets.push(button);
    let mut p = ProfileSpec::default_for("p/default".parse().unwrap());
    p.default = true;
    p.slots.push(SlotSpec {
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
        profiles: vec![p],
        ..Catalog::default()
    }
}
fn info(id: u64) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(id),
        name: format!("headless:{id}").parse().unwrap(),
        kind: SurfaceKind::Headless,
        width_px: 500,
        height_px: 50,
        px_per_mm: 5.0,
        caps: SurfaceCaps {
            modes: true,
            inject_keys: true,
            ..SurfaceCaps::default()
        },
    }
}
fn call(out: &[Output]) -> Option<(InstanceId, CallId, LogicCall)> {
    out.iter().find_map(|o| match o {
        Output::CallLogic {
            instance,
            call,
            invocation,
        } => Some((*instance, *call, invocation.clone())),
        _ => None,
    })
}
fn reply(c: &mut Core, at: u64, out: &[Output], update: LogicUpdate) -> Vec<Output> {
    let (instance, call, _) = call(out).unwrap();
    c.handle(
        Timestamp(at),
        Input::LogicUpdate {
            instance,
            call,
            update,
        },
    )
}
fn drain(c: &mut Core, at: u64, mut out: Vec<Output>) -> Vec<Output> {
    let mut all = out.clone();
    while call(&out).is_some() {
        out = reply(c, at, &out, LogicUpdate::default());
        all.extend(out.clone());
    }
    all
}
fn setup(config: CoreConfig) -> (Core, InstanceId) {
    let mut c = Core::new(config);
    c.handle(Timestamp(0), Input::Catalog(catalog()));
    add(&mut c, 1);
    let id = c.snapshot().instances[0].id;
    (c, id)
}
fn add(c: &mut Core, n: u64) {
    c.handle(Timestamp(0), Input::SurfaceAdded(info(n)));
    c.handle(
        Timestamp(0),
        Input::Mode {
            surface: SurfaceId(n),
            mode: Mode::Display,
        },
    );
    let id = c
        .snapshot()
        .surfaces
        .iter()
        .find(|s| s.info.id == SurfaceId(n))
        .unwrap()
        .scene
        .items[0]
        .instance;
    let init = c.handle(
        Timestamp(0),
        Input::LogicDescribed {
            persisted: vec![],
            instance: id,
            description: LogicDescription {
                actions: vec![(
                    "press".into(),
                    ActionDescription {
                        kinds: vec![ActionKind::Pressed],
                        value: None,
                    },
                )],
                state: vec![],
            },
        },
    );
    let out = reply(c, 0, &init, LogicUpdate::default());
    drain(c, 0, out);
    layout(c, 0, n);
}
fn layout(c: &mut Core, at: u64, n: u64) {
    let s = c
        .snapshot()
        .surfaces
        .into_iter()
        .find(|s| s.info.id == SurfaceId(n))
        .unwrap();
    c.handle(
        Timestamp(at),
        Input::FrameDone {
            surface: s.info.id,
            generation: s.generation,
        },
    );
    let s = c
        .snapshot()
        .surfaces
        .into_iter()
        .find(|s| s.info.id == SurfaceId(n))
        .unwrap();
    c.handle(
        Timestamp(at),
        Input::Layout {
            surface: s.info.id,
            generation: s.generation,
            hit_map: vec![HitTarget {
                instance: s.scene.items[0].instance,
                widget: "button".parse().unwrap(),
                plane: ScenePlane::Profile,
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 50.0,
                },
                record_key: None,
            }],
            representations: vec![],
            scrubbers: vec![],
            slots: vec![],
            presentation: None,
        },
    );
}
fn submit_on(c: &mut Core, at: u64, n: u64, update: LogicUpdate) -> Vec<Output> {
    let down = c.handle(
        Timestamp(at),
        Input::Touch {
            surface: SurfaceId(n),
            contact: ContactId::new(1).unwrap(),
            phase: TouchPhase::Down,
            x: 20.0,
            y: 20.0,
        },
    );
    let out = reply(c, at, &down, update);
    c.handle(
        Timestamp(at),
        Input::Touch {
            surface: SurfaceId(n),
            contact: ContactId::new(1).unwrap(),
            phase: TouchPhase::Up,
            x: 20.0,
            y: 20.0,
        },
    );
    drain(c, at, out)
}
fn submit(c: &mut Core, at: u64, update: LogicUpdate) -> Vec<Output> {
    submit_on(c, at, 1, update)
}
fn req(tag: &str, effect: Effect) -> EffectRequest {
    EffectRequest {
        tag: tag.into(),
        effect,
        lifetime: None,
        coalesce: None,
        timeout_ms: None,
    }
}
fn starts(out: &[Output]) -> Vec<(RequestId, Effect)> {
    out.iter()
        .filter_map(|o| match o {
            Output::StartEffect {
                request, effect, ..
            } => Some((*request, effect.clone())),
            _ => None,
        })
        .collect()
}
fn done(c: &mut Core, at: u64, id: RequestId) -> Vec<Output> {
    c.handle(
        Timestamp(at),
        Input::EffectDone {
            request: id,
            result: Ok(EffectResult::None),
        },
    )
}
#[test]
fn slots_queue_requests_and_completion_starts_next() {
    let (mut c, _) = setup(CoreConfig {
        effect_slots: 1,
        ..CoreConfig::default()
    });
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            requests: vec![
                req("first", Effect::MediaNext),
                req("second", Effect::MediaPrevious),
            ],
            ..LogicUpdate::default()
        },
    );
    assert_eq!(starts(&out).len(), 1);
    assert_eq!(starts(&out)[0].1, Effect::MediaNext);
    let out = done(&mut c, 2, starts(&out)[0].0);
    assert_eq!(starts(&out)[0].1, Effect::MediaPrevious);
    assert!(
        matches!(call(&out).unwrap().2, LogicCall::Handle(LogicEvent::EffectDone { tag, result: Ok(_) }) if tag == "first")
    );
}
#[test]
fn coalesce_preserves_running_and_executes_only_latest_pending() {
    let (mut c, _) = setup(CoreConfig::default());
    let volume = |tag: &str, level| EffectRequest {
        coalesce: Some("volume".into()),
        ..req(
            tag,
            Effect::AudioSetVolume {
                target: AudioTarget::DefaultSink,
                level,
            },
        )
    };
    let first = submit(
        &mut c,
        1,
        LogicUpdate {
            requests: vec![volume("a", 0.1)],
            ..LogicUpdate::default()
        },
    );
    let middle = submit(
        &mut c,
        2,
        LogicUpdate {
            requests: vec![volume("b", 0.2)],
            ..LogicUpdate::default()
        },
    );
    assert!(starts(&middle).is_empty());
    let last = submit(
        &mut c,
        3,
        LogicUpdate {
            requests: vec![volume("c", 0.3)],
            ..LogicUpdate::default()
        },
    );
    assert!(starts(&last).is_empty());
    assert!(
        !last.iter().any(
            |o| matches!(o, Output::CancelEffect { request } if *request == starts(&first)[0].0)
        )
    );
    let out = done(&mut c, 4, starts(&first)[0].0);
    assert!(matches!(
        starts(&out)[0].1,
        Effect::AudioSetVolume { level: 0.3, .. }
    ));
}
#[test]
fn committed_queue_survives_dispose_and_results_are_discarded() {
    let (mut c, _) = setup(CoreConfig {
        effect_slots: 1,
        ..CoreConfig::default()
    });
    let mut spawn = req(
        "spawn",
        Effect::SpawnRun(SpawnSpec::new(vec!["sleep".into(), "1".into()])),
    );
    spawn.lifetime = Some(Lifetime::Committed);
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            requests: vec![spawn, req("next", Effect::MediaNext)],
            ..LogicUpdate::default()
        },
    );
    let running = starts(&out)[0].0;
    let removed = c.handle(Timestamp(2), Input::SurfaceRemoved(SurfaceId(1)));
    assert!(!removed.contains(&Output::CancelEffect { request: running }));
    let out = done(&mut c, 3, running);
    assert_eq!(starts(&out)[0].1, Effect::MediaNext);
    assert!(call(&out).is_none());
    let out = done(&mut c, 4, starts(&out)[0].0);
    assert!(call(&out).is_none());
}
#[test]
fn explicit_cancel_cancels_running_committed_and_stale_done_is_ignored() {
    let (mut c, _) = setup(CoreConfig::default());
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            requests: vec![req("write", Effect::MediaNext)],
            ..LogicUpdate::default()
        },
    );
    let id = starts(&out)[0].0;
    let out = submit(
        &mut c,
        2,
        LogicUpdate {
            cancel: vec!["write".into()],
            ..LogicUpdate::default()
        },
    );
    assert!(out.contains(&Output::CancelEffect { request: id }));
    assert!(out.iter().any(|o| matches!(o, Output::CallLogic { invocation: LogicCall::Handle(LogicEvent::EffectDone { tag, result: Err(EffectError::Cancelled) }), .. } if tag == "write")));
    assert!(call(&done(&mut c, 3, id)).is_none());
}
#[test]
fn hiding_cancels_until_hidden_but_not_committed() {
    let (mut c, _) = setup(CoreConfig::default());
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            requests: vec![
                req("wait", Effect::TimerAfter { ms: 500 }),
                req("write", Effect::MediaNext),
            ],
            ..LogicUpdate::default()
        },
    );
    let ids = starts(&out);
    let out = c.handle(
        Timestamp(2),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Native,
        },
    );
    assert!(out.contains(&Output::CancelEffect { request: ids[0].0 }));
    assert!(!out.contains(&Output::CancelEffect { request: ids[1].0 }));
}
#[test]
fn timeout_cancels_running_and_delivers_timeout_once() {
    let (mut c, _) = setup(CoreConfig::default());
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            requests: vec![EffectRequest {
                timeout_ms: Some(10),
                ..req("write", Effect::MediaNext)
            }],
            ..LogicUpdate::default()
        },
    );
    let id = starts(&out)[0].0;
    let timer = out
        .iter()
        .find_map(|o| match o {
            Output::SetTimer {
                id,
                at: Timestamp(11),
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    let out = c.handle(Timestamp(11), Input::Timer { id: timer });
    assert!(out.contains(&Output::CancelEffect { request: id }));
    assert!(matches!(
        call(&out).unwrap().2,
        LogicCall::Handle(LogicEvent::EffectDone {
            result: Err(EffectError::Timeout),
            ..
        })
    ));
    drain(&mut c, 11, out);
    assert!(done(&mut c, 12, id).is_empty());
}
#[test]
fn spawn_limits_do_not_block_nonspawn_requests_and_freed_slot_serves_other_instance() {
    let (mut c, _) = setup(CoreConfig {
        spawn_slots: 1,
        session_spawn_slots: 1,
        ..CoreConfig::default()
    });
    add(&mut c, 2);
    let spawn = |tag| req(tag, Effect::SpawnRun(SpawnSpec::new(vec!["true".into()])));
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            requests: vec![spawn("a"), spawn("b"), req("media", Effect::MediaNext)],
            ..LogicUpdate::default()
        },
    );
    assert_eq!(starts(&out).len(), 2);
    let out2 = submit_on(
        &mut c,
        2,
        2,
        LogicUpdate {
            requests: vec![spawn("c")],
            ..LogicUpdate::default()
        },
    );
    assert!(starts(&out2).is_empty());
    let out = done(&mut c, 3, starts(&out)[0].0);
    assert_eq!(starts(&out).len(), 1);
    let out = drain(&mut c, 3, out);
    let next = starts(&out)[0].0;
    let out = done(&mut c, 4, next);
    assert_eq!(
        starts(&out)[0].0.instance,
        c.snapshot().surfaces[1].scene.items[0].instance
    );
}
#[test]
fn key_down_releases_on_hidden_and_pending_down_does_not_press_hidden_owner() {
    let (mut c, _) = setup(CoreConfig {
        effect_slots: 1,
        ..CoreConfig::default()
    });
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            requests: vec![req("down", Effect::KeyDown { key: SystemKey::F1 })],
            ..LogicUpdate::default()
        },
    );
    assert!(out.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Pressed
    }));
    let out = submit(
        &mut c,
        2,
        LogicUpdate {
            requests: vec![
                req("blocker", Effect::MediaNext),
                req("late-down", Effect::KeyDown { key: SystemKey::F2 }),
            ],
            ..LogicUpdate::default()
        },
    );
    let blocker = starts(&out)[0].0;
    let hidden = c.handle(
        Timestamp(3),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Native,
        },
    );
    assert!(hidden.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    drain(&mut c, 3, hidden);
    let out = done(&mut c, 4, blocker);
    assert!(!out.iter().any(|o| matches!(
        o,
        Output::SendKey {
            phase: KeyPhase::Pressed,
            ..
        }
    )));
    let out = drain(&mut c, 4, out);
    assert!(out.iter().any(|o| matches!(o, Output::CallLogic { invocation: LogicCall::Handle(LogicEvent::EffectDone { tag, result: Err(EffectError::Unavailable) }), .. } if tag == "late-down")));
}

fn sub(tag: &str, background: bool) -> SubscriptionSpec {
    SubscriptionSpec {
        tag: tag.into(),
        kind: SubscriptionKind::AudioWatch {
            target: AudioTarget::DefaultSink,
        },
        background,
    }
}
fn subscriptions(out: &[Output]) -> Vec<SubscriptionId> {
    out.iter()
        .filter_map(|o| match o {
            Output::Subscribe { subscription, .. } => Some(*subscription),
            _ => None,
        })
        .collect()
}
#[test]
fn subscriptions_diff_preserves_identity_and_data_uses_tag() {
    let (mut c, _) = setup(CoreConfig::default());
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            subscriptions: Some(vec![sub("audio", false)]),
            ..LogicUpdate::default()
        },
    );
    let id = subscriptions(&out)[0];
    let out = submit(
        &mut c,
        2,
        LogicUpdate {
            subscriptions: Some(vec![sub("audio", true), sub("other", false)]),
            ..LogicUpdate::default()
        },
    );
    assert_eq!(subscriptions(&out).len(), 1);
    assert!(!out.contains(&Output::Unsubscribe { subscription: id }));
    let data = SubscriptionData::Audio(AudioState {
        available: true,
        volume: 0.5,
        muted: false,
        name: "device".into(),
    });
    let out = c.handle(
        Timestamp(3),
        Input::SubscriptionData {
            subscription: id,
            data: data.clone(),
        },
    );
    assert!(
        matches!(call(&out).unwrap().2, LogicCall::Handle(LogicEvent::SubscriptionData { tag, data: actual }) if tag == "audio" && actual == data)
    );
    drain(&mut c, 3, out);
    let out = submit(
        &mut c,
        4,
        LogicUpdate {
            subscriptions: Some(vec![]),
            ..LogicUpdate::default()
        },
    );
    assert_eq!(
        out.iter()
            .filter(|o| matches!(o, Output::Unsubscribe { .. }))
            .count(),
        2
    );
    assert!(
        c.handle(
            Timestamp(5),
            Input::SubscriptionData {
                subscription: id,
                data
            }
        )
        .is_empty()
    );
}
#[test]
fn initial_hidden_only_starts_background_and_show_starts_visible_subscription() {
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::Catalog(catalog()));
    c.handle(Timestamp(0), Input::SurfaceAdded(info(1)));
    let id = c.snapshot().instances[0].id;
    let init = c.handle(
        Timestamp(0),
        Input::LogicDescribed {
            persisted: vec![],
            instance: id,
            description: LogicDescription {
                actions: vec![(
                    "press".into(),
                    ActionDescription {
                        kinds: vec![ActionKind::Pressed],
                        value: None,
                    },
                )],
                state: vec![],
            },
        },
    );
    let out = reply(
        &mut c,
        0,
        &init,
        LogicUpdate {
            subscriptions: Some(vec![sub("normal", false), sub("background", true)]),
            ..LogicUpdate::default()
        },
    );
    assert_eq!(subscriptions(&out).len(), 1);
    let background = subscriptions(&out)[0];
    let shown = c.handle(
        Timestamp(1),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Display,
        },
    );
    assert_eq!(subscriptions(&shown).len(), 1);
    let normal = subscriptions(&shown)[0];
    drain(&mut c, 1, shown);
    let hidden = c.handle(
        Timestamp(2),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Native,
        },
    );
    assert!(hidden.contains(&Output::Unsubscribe {
        subscription: normal
    }));
    assert!(!hidden.contains(&Output::Unsubscribe {
        subscription: background
    }));
    drain(&mut c, 2, hidden);
    let shown = c.handle(
        Timestamp(3),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Display,
        },
    );
    assert_eq!(subscriptions(&shown).len(), 1);
    assert_ne!(subscriptions(&shown)[0], normal);
}
#[test]
fn subscription_lost_retries_with_backoff_fresh_id_and_ignores_old_data() {
    let (mut c, _) = setup(CoreConfig::default());
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            subscriptions: Some(vec![sub("audio", false)]),
            ..LogicUpdate::default()
        },
    );
    let first = subscriptions(&out)[0];
    let lost = c.handle(
        Timestamp(2),
        Input::SubscriptionLost {
            subscription: first,
            error: EffectError::Unavailable,
        },
    );
    assert!(
        matches!(call(&lost).unwrap().2, LogicCall::Handle(LogicEvent::SubscriptionLost { tag, .. }) if tag == "audio")
    );
    let timer = lost
        .iter()
        .find_map(|o| match o {
            Output::SetTimer {
                id,
                at: Timestamp(1002),
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    drain(&mut c, 2, lost);
    assert!(
        c.handle(
            Timestamp(3),
            Input::SubscriptionData {
                subscription: first,
                data: SubscriptionData::Tick
            }
        )
        .is_empty()
    );
    let retry = c.handle(Timestamp(1002), Input::Timer { id: timer });
    let second = subscriptions(&retry)[0];
    assert_ne!(second, first);
    let lost = c.handle(
        Timestamp(1003),
        Input::SubscriptionLost {
            subscription: second,
            error: EffectError::Unavailable,
        },
    );
    let timer = lost
        .iter()
        .find_map(|o| match o {
            Output::SetTimer {
                id,
                at: Timestamp(3003),
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    drain(&mut c, 1003, lost);
    let retry = c.handle(Timestamp(3003), Input::Timer { id: timer });
    let third = subscriptions(&retry)[0];
    assert_ne!(second, third);
    let removed = c.handle(Timestamp(3004), Input::SurfaceRemoved(SurfaceId(1)));
    assert!(removed.contains(&Output::Unsubscribe {
        subscription: third
    }));
    assert!(
        c.handle(
            Timestamp(4000),
            Input::SubscriptionLost {
                subscription: third,
                error: EffectError::Unavailable
            }
        )
        .is_empty()
    );
}
#[test]
fn spawn_subscription_shares_process_slots_with_requests() {
    let (mut c, _) = setup(CoreConfig {
        spawn_slots: 1,
        session_spawn_slots: 1,
        ..CoreConfig::default()
    });
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            requests: vec![req(
                "run",
                Effect::SpawnRun(SpawnSpec::new(vec!["true".into()])),
            )],
            subscriptions: Some(vec![SubscriptionSpec {
                tag: "lines".into(),
                kind: SubscriptionKind::SpawnLines(SpawnSpec::new(vec!["true".into()])),
                background: false,
            }]),
            ..LogicUpdate::default()
        },
    );
    assert_eq!(starts(&out).len(), 1);
    assert!(subscriptions(&out).is_empty());
    let out = done(&mut c, 2, starts(&out)[0].0);
    assert_eq!(subscriptions(&out).len(), 1);
    let lines = subscriptions(&out)[0];
    drain(&mut c, 2, out);
    let out = submit(
        &mut c,
        3,
        LogicUpdate {
            requests: vec![req(
                "later",
                Effect::SpawnRun(SpawnSpec::new(vec!["true".into()])),
            )],
            ..LogicUpdate::default()
        },
    );
    assert!(starts(&out).is_empty());
    let out = submit(
        &mut c,
        4,
        LogicUpdate {
            subscriptions: Some(vec![]),
            ..LogicUpdate::default()
        },
    );
    assert!(out.contains(&Output::Unsubscribe {
        subscription: lines
    }));
    assert_eq!(starts(&out).len(), 1);
}
#[test]
fn subscription_kind_mismatch_and_invalid_desired_set_preserve_existing_subscription() {
    let (mut c, _) = setup(CoreConfig::default());
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            subscriptions: Some(vec![sub("audio", false)]),
            ..LogicUpdate::default()
        },
    );
    let id = subscriptions(&out)[0];
    let out = c.handle(
        Timestamp(2),
        Input::SubscriptionData {
            subscription: id,
            data: SubscriptionData::Tick,
        },
    );
    assert!(call(&out).is_none());
    let out = submit(
        &mut c,
        3,
        LogicUpdate {
            subscriptions: Some(vec![sub("duplicate", false), sub("duplicate", true)]),
            ..LogicUpdate::default()
        },
    );
    assert!(subscriptions(&out).is_empty());
    assert!(!out.contains(&Output::Unsubscribe { subscription: id }));
    let out = c.handle(
        Timestamp(4),
        Input::SubscriptionData {
            subscription: id,
            data: SubscriptionData::Audio(AudioState {
                available: true,
                volume: 0.5,
                muted: false,
                name: "device".into(),
            }),
        },
    );
    assert!(call(&out).is_some());
}

#[test]
fn default_eight_effect_slots_queue_the_ninth_request() {
    let (mut c, _) = setup(CoreConfig::default());
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            requests: (0..9)
                .map(|n| req(&format!("request-{n}"), Effect::MediaNext))
                .collect(),
            ..LogicUpdate::default()
        },
    );
    assert_eq!(starts(&out).len(), 8);
    let out = done(&mut c, 2, starts(&out)[0].0);
    assert_eq!(starts(&out).len(), 1);
}
#[test]
fn default_spawn_limits_apply_across_twenty_four_instances() {
    let mut cat = catalog();
    cat.profiles[0].slots = (0..3)
        .map(|n| SlotSpec {
            id: format!("slot-{n}").parse().unwrap(),
            candidates: vec!["p/main".parse().unwrap()],
        })
        .collect();
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::Catalog(cat));
    for n in 1..=8 {
        c.handle(Timestamp(0), Input::SurfaceAdded(info(n)));
        c.handle(
            Timestamp(0),
            Input::Mode {
                surface: SurfaceId(n),
                mode: Mode::Display,
            },
        );
    }
    let owners: Vec<_> = c
        .snapshot()
        .surfaces
        .iter()
        .flat_map(|s| s.scene.items.iter().map(|i| (s.info.id, i.instance)))
        .collect();
    assert_eq!(owners.len(), 24);
    for (_, id) in &owners {
        let init = c.handle(
            Timestamp(0),
            Input::LogicDescribed {
                persisted: vec![],
                instance: *id,
                description: LogicDescription {
                    actions: vec![(
                        "press".into(),
                        ActionDescription {
                            kinds: vec![ActionKind::Pressed],
                            value: None,
                        },
                    )],
                    state: vec![],
                },
            },
        );
        let out = reply(&mut c, 0, &init, LogicUpdate::default());
        drain(&mut c, 0, out);
    }
    let mut all = vec![];
    for (surface, id) in owners {
        let generation = c
            .snapshot()
            .surfaces
            .iter()
            .find(|s| s.info.id == surface)
            .unwrap()
            .generation;
        c.handle(
            Timestamp(1),
            Input::Layout {
                surface,
                generation,
                hit_map: vec![HitTarget {
                    instance: id,
                    widget: "button".parse().unwrap(),
                    plane: ScenePlane::Profile,
                    rect: Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0,
                        height: 50.0,
                    },
                    record_key: None,
                }],
                representations: vec![],
                scrubbers: vec![],
                slots: vec![],
                presentation: None,
            },
        );
        let mut requests: Vec<_> = (0..5)
            .map(|n| {
                req(
                    &format!("spawn-{n}"),
                    Effect::SpawnRun(SpawnSpec::new(vec!["true".into()])),
                )
            })
            .collect();
        requests.push(req("media", Effect::MediaNext));
        all.extend(submit_on(
            &mut c,
            1,
            surface.0,
            LogicUpdate {
                requests,
                ..LogicUpdate::default()
            },
        ));
    }
    let started = starts(&all);
    let spawns: Vec<_> = started
        .iter()
        .filter(|(_, e)| matches!(e, Effect::SpawnRun(_)))
        .collect();
    assert_eq!(spawns.len(), 64);
    assert_eq!(
        started
            .iter()
            .filter(|(_, e)| matches!(e, Effect::MediaNext))
            .count(),
        24
    );
    let first = spawns[0].0.instance;
    assert_eq!(
        spawns.iter().filter(|(id, _)| id.instance == first).count(),
        4
    );
    let out = done(&mut c, 2, spawns[0].0);
    assert_eq!(starts(&out).len(), 1);
}
#[test]
fn committed_cancel_and_replace_never_executes_replaced_pending_request() {
    let (mut c, _) = setup(CoreConfig {
        effect_slots: 1,
        ..CoreConfig::default()
    });
    let pending = |tag| EffectRequest {
        coalesce: Some("write".into()),
        ..req(tag, Effect::MediaPrevious)
    };
    let first = submit(
        &mut c,
        1,
        LogicUpdate {
            requests: vec![req("blocker", Effect::MediaNext), pending("old")],
            ..LogicUpdate::default()
        },
    );
    let blocker = starts(&first)[0].0;
    let out = submit(
        &mut c,
        2,
        LogicUpdate {
            cancel: vec!["blocker".into()],
            requests: vec![pending("new")],
            ..LogicUpdate::default()
        },
    );
    assert!(out.contains(&Output::CancelEffect { request: blocker }));
    assert_eq!(starts(&out).len(), 1);
    let out = done(&mut c, 3, starts(&out)[0].0);
    assert!(
        matches!(call(&out).unwrap().2, LogicCall::Handle(LogicEvent::EffectDone { tag, .. }) if tag == "new")
    );
}
#[test]
fn key_tap_does_not_release_an_existing_logic_key_hold() {
    let (mut c, _) = setup(CoreConfig::default());
    submit(
        &mut c,
        1,
        LogicUpdate {
            requests: vec![req("down", Effect::KeyDown { key: SystemKey::F1 })],
            ..LogicUpdate::default()
        },
    );
    let tapped = submit(
        &mut c,
        2,
        LogicUpdate {
            requests: vec![req("tap", Effect::KeyTap { key: SystemKey::F1 })],
            ..LogicUpdate::default()
        },
    );
    assert!(!tapped.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    let up = submit(
        &mut c,
        3,
        LogicUpdate {
            requests: vec![req("up", Effect::KeyUp { key: SystemKey::F1 })],
            ..LogicUpdate::default()
        },
    );
    assert!(up.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
}
#[test]
fn queued_committed_key_tap_runs_after_profile_dispose_but_key_down_does_not() {
    let (mut c, id) = setup(CoreConfig {
        effect_slots: 1,
        ..CoreConfig::default()
    });
    let first = submit(
        &mut c,
        1,
        LogicUpdate {
            requests: vec![
                req("block", Effect::MediaNext),
                req("down", Effect::KeyDown { key: SystemKey::F1 }),
                req("tap", Effect::KeyTap { key: SystemKey::F2 }),
            ],
            ..LogicUpdate::default()
        },
    );
    c.handle(
        Timestamp(2),
        Input::Catalog(Catalog {
            profiles: vec![],
            ..catalog()
        }),
    );
    assert!(c.snapshot().instances.is_empty());
    let out = done(&mut c, 3, starts(&first)[0].0);
    assert!(call(&out).is_none());
    assert!(!out.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Pressed
    }));
    assert!(out.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F2,
        phase: KeyPhase::Pressed
    }));
    assert!(out.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F2,
        phase: KeyPhase::Released
    }));
    assert!(!c.snapshot().instances.iter().any(|i| i.id == id));
}

#[test]
fn fault_releases_keys_stops_background_subscription_and_preserves_committed_request() {
    let (mut c, id) = setup(CoreConfig::default());
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            requests: vec![
                req("key", Effect::KeyDown { key: SystemKey::F1 }),
                req("write", Effect::MediaNext),
            ],
            subscriptions: Some(vec![sub("audio", true)]),
            ..LogicUpdate::default()
        },
    );
    let running = starts(&out)[0].0;
    let subscribed = subscriptions(&out)[0];
    let down = c.handle(
        Timestamp(2),
        Input::Touch {
            surface: SurfaceId(1),
            contact: ContactId::new(1).unwrap(),
            phase: TouchPhase::Down,
            x: 20.0,
            y: 20.0,
        },
    );
    let (_, call_id, _) = call(&down).unwrap();
    let fault = c.handle(
        Timestamp(3),
        Input::LogicFault {
            instance: id,
            call: call_id,
            reason: "trap".into(),
        },
    );
    assert!(fault.contains(&Output::SendKey {
        surface: SurfaceId(1),
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    assert!(fault.contains(&Output::Unsubscribe {
        subscription: subscribed
    }));
    assert!(!fault.contains(&Output::CancelEffect { request: running }));
    let timer = fault
        .iter()
        .find_map(|o| match o {
            Output::SetTimer {
                id,
                at: Timestamp(1003),
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    let retry = c.handle(Timestamp(1003), Input::Timer { id: timer });
    let new_id = retry
        .iter()
        .find_map(|o| match o {
            Output::CreateInstance { instance, .. } => Some(*instance),
            _ => None,
        })
        .unwrap();
    assert_ne!(id, new_id);
    assert!(call(&done(&mut c, 1004, running)).is_none());
    assert!(
        c.handle(
            Timestamp(1005),
            Input::SubscriptionLost {
                subscription: subscribed,
                error: EffectError::Unavailable
            }
        )
        .is_empty()
    );
}
#[test]
fn successful_subscription_data_resets_backoff_and_hidden_retry_resumes_at_deadline() {
    let (mut c, _) = setup(CoreConfig::default());
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            subscriptions: Some(vec![sub("audio", false)]),
            ..LogicUpdate::default()
        },
    );
    let id = subscriptions(&out)[0];
    let out = c.handle(
        Timestamp(2),
        Input::SubscriptionLost {
            subscription: id,
            error: EffectError::Unavailable,
        },
    );
    let timer = out
        .iter()
        .find_map(|o| match o {
            Output::SetTimer {
                id,
                at: Timestamp(1002),
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    drain(&mut c, 2, out);
    let out = c.handle(
        Timestamp(3),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Native,
        },
    );
    drain(&mut c, 3, out);
    let out = c.handle(
        Timestamp(4),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Display,
        },
    );
    assert!(subscriptions(&out).is_empty());
    let new_timer = out
        .iter()
        .find_map(|o| match o {
            Output::SetTimer {
                id,
                at: Timestamp(1002),
            } => Some(*id),
            _ => None,
        })
        .unwrap();
    assert_ne!(timer, new_timer);
    drain(&mut c, 4, out);
    let out = c.handle(Timestamp(1002), Input::Timer { id: new_timer });
    let id = subscriptions(&out)[0];
    let out = c.handle(
        Timestamp(1003),
        Input::SubscriptionData {
            subscription: id,
            data: SubscriptionData::Audio(AudioState {
                available: true,
                volume: 0.5,
                muted: false,
                name: "device".into(),
            }),
        },
    );
    drain(&mut c, 1003, out);
    let out = c.handle(
        Timestamp(1004),
        Input::SubscriptionLost {
            subscription: id,
            error: EffectError::Unavailable,
        },
    );
    assert!(out.iter().any(|o| matches!(
        o,
        Output::SetTimer {
            at: Timestamp(2004),
            ..
        }
    )));
}
#[test]
fn layer_preserves_underlying_subscriptions_and_committed_effects() {
    let (mut c, _) = setup(CoreConfig::default());
    let out = submit(
        &mut c,
        1,
        LogicUpdate {
            requests: vec![req("write", Effect::MediaNext)],
            subscriptions: Some(vec![sub("audio", false)]),
            ..LogicUpdate::default()
        },
    );
    let effect = starts(&out)[0].0;
    let sub = subscriptions(&out)[0];
    let mut cat = catalog();
    cat.sets
        .push(SetSpec::default_for("p/layer".parse().unwrap()));
    cat.bindings.push(Binding {
        id: "layer".into(),
        trigger: Trigger::Key(KeyTrigger {
            keys: vec![WatchKey::Fn],
            gesture: KeyGesture::WhileHeld,
        }),
        when: None,
        action: LayerAction {
            operation: LayerOperation::Layer("p/layer".parse().unwrap()),
            surfaces: LayerSurfaces::Source,
        },
    });
    c.handle(Timestamp(2), Input::Catalog(cat));
    let out = c.handle(
        Timestamp(3),
        Input::Key {
            source: KeySource::Desktop,
            code: WatchKey::Fn,
            phase: KeyPhase::Pressed,
        },
    );
    assert!(!out.contains(&Output::Unsubscribe { subscription: sub }));
    assert!(!out.contains(&Output::CancelEffect { request: effect }));
    assert!(
        c.snapshot().surfaces[0]
            .scene
            .items
            .iter()
            .any(|i| matches!(i.plane, ScenePlane::Layer(_)))
    );
}
