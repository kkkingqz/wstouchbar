use touchbar_core::*;
use touchbar_model::*;
use touchbar_testkit::*;

fn controls() -> Catalog {
    catalog()
        .plugin("controls")
        .set("volume", |s| {
            s.slider("level");
            s.button("microphone");
        })
        .profile("default", |p| {
            p.default().slot("main", &["controls/volume"]);
        })
        .build()
}
fn volume_logic() -> FakeLogic {
    let mut logic = FakeLogic::default();
    logic.register("controls/volume", None, |_, call| {
        let mut update = LogicUpdate::default();
        if let LogicCall::Handle(LogicEvent::Action(a)) = call {
            let effect = if a.action == "level" && a.kind == ActionKind::ValueChanged {
                Some(Effect::AudioSetVolume {
                    target: AudioTarget::DefaultSink,
                    level: match a.value {
                        Some(Scalar::Number(v)) => v,
                        _ => 0.0,
                    },
                })
            } else if a.action == "microphone" && a.kind == ActionKind::Activated {
                Some(Effect::AudioSetMute {
                    target: AudioTarget::DefaultSource,
                    value: MuteValue::Toggle,
                })
            } else {
                None
            };
            if let Some(effect) = effect {
                update.requests.push(EffectRequest {
                    tag: "write".into(),
                    effect,
                    lifetime: None,
                    coalesce: Some("write".into()),
                    timeout_ms: None,
                });
            }
        }
        LogicReaction::Update(update)
    });
    logic
}
fn setup(logic: FakeLogic) -> Harness {
    let mut h = Harness::new(CoreConfig::default());
    h.logic = logic;
    h.send(Input::Catalog(controls()));
    h.send(Input::SurfaceAdded(headless(1)));
    h
}
#[test]
fn virtual_clock_fires_the_nearest_core_timer_without_real_sleep() {
    let mut logic = FakeLogic::default();
    logic.register("controls/volume", None, |_, call| {
        if matches!(call, LogicCall::Init { .. }) {
            LogicReaction::Fault("trap".into())
        } else {
            LogicReaction::Update(LogicUpdate::default())
        }
    });
    let mut h = setup(logic);
    let first = h.snapshot().instances[0].id;
    assert_eq!(h.snapshot().instances[0].status, InstanceStatus::Faulted);
    h.advance_ms(999);
    assert_eq!(h.snapshot().instances[0].id, first);
    h.advance_ms(1);
    assert_ne!(h.snapshot().instances[0].id, first);
    assert_eq!(h.now(), Timestamp(1000));
    assert!(
        h.trace
            .iter()
            .any(|e| e.at == Timestamp(1000) && matches!(e.input, Input::Timer { .. }))
    );
}
#[test]
fn delayed_logic_and_hang_obey_virtual_deadlines_and_one_in_flight_call() {
    let mut logic = FakeLogic::default();
    logic.register("controls/volume", None, |_, call| {
        if matches!(call, LogicCall::Init { .. }) {
            LogicReaction::Delayed {
                ms: 10,
                update: LogicUpdate::default(),
            }
        } else {
            LogicReaction::Update(LogicUpdate::default())
        }
    });
    let mut h = setup(logic);
    assert_eq!(h.snapshot().instances[0].status, InstanceStatus::Loading);
    h.advance_ms(9);
    assert_eq!(h.logic.calls.len(), 1);
    h.advance_ms(1);
    assert_eq!(h.snapshot().instances[0].status, InstanceStatus::Visible);
    assert!(
        h.logic
            .calls
            .iter()
            .any(|(_, _, call)| matches!(call, LogicCall::Handle(LogicEvent::ContextChanged(_))))
    );
    let mut logic = FakeLogic::default();
    logic.register("controls/volume", None, |_, _| LogicReaction::Hang);
    let mut h = setup(logic);
    h.advance_ms(49);
    assert_eq!(h.snapshot().instances[0].status, InstanceStatus::Loading);
    h.advance_ms(1);
    assert_eq!(h.snapshot().instances[0].status, InstanceStatus::Faulted);
    assert!(
        h.trace.iter().any(
            |e| matches!(&e.input,Input::LogicFault {reason,..} if reason.contains("timeout"))
        )
    );
}
#[test]
fn effects_finish_after_configured_delay_and_replay_has_no_fake_dependencies() {
    let mut h = setup(volume_logic());
    h.effects.delay_ms = 100;
    h.tap(SurfaceId(1), "microphone");
    assert_eq!(h.effects.started.len(), 1);
    assert!(h.snapshot().requests[0].running);
    h.advance_ms(99);
    assert_eq!(h.snapshot().requests.len(), 1);
    h.advance_ms(1);
    assert!(h.snapshot().requests.is_empty());
    let dump = ReplayDump::from_json(&h.dump().to_json().unwrap()).unwrap();
    assert_eq!(dump.replay().unwrap().core.snapshot(), h.snapshot());
}
#[test]
fn cancelled_effect_does_not_deliver_an_executor_completion() {
    let mut logic = FakeLogic::default();
    logic.register("controls/volume", None, |_, call| {
        let mut u = LogicUpdate::default();
        if matches!(call, LogicCall::Init { .. }) {
            u.requests.push(EffectRequest {
                tag: "job".into(),
                effect: Effect::TimerAfter { ms: 100 },
                lifetime: Some(Lifetime::UntilHidden),
                coalesce: None,
                timeout_ms: None,
            });
        }
        LogicReaction::Update(u)
    });
    let mut h = setup(logic);
    assert_eq!(h.effects.started.len(), 1);
    h.send(Input::SurfaceRemoved(SurfaceId(1)));
    h.advance_ms(100);
    assert!(
        h.trace
            .iter()
            .all(|e| !matches!(e.input, Input::EffectDone { .. }))
    );
    assert_eq!(h.effects.cancelled.len(), 1);
}
#[test]
fn subscription_ticks_and_explicit_data_follow_virtual_time_and_unsubscribe() {
    let mut logic = FakeLogic::default();
    logic.register("controls/volume", None, |_, call| {
        let mut u = LogicUpdate::default();
        if matches!(call, LogicCall::Init { .. }) {
            u.subscriptions = Some(vec![SubscriptionSpec {
                tag: "clock".into(),
                kind: SubscriptionKind::TimerInterval { ms: 20 },
                background: false,
            }]);
        }
        LogicReaction::Update(u)
    });
    let mut h = setup(logic);
    let sub = h.snapshot().subscriptions[0].active.unwrap();
    h.advance_ms(19);
    assert!(
        h.trace
            .iter()
            .all(|e| !matches!(e.input, Input::SubscriptionData { .. }))
    );
    h.advance_ms(1);
    assert!(h.trace.iter().any(|e| e.at == Timestamp(20)
        && matches!(
            e.input,
            Input::SubscriptionData {
                data: SubscriptionData::Tick,
                ..
            }
        )));
    h.subscription_after(sub, SubscriptionData::Tick, 5);
    h.advance_ms(5);
    h.send(Input::SurfaceRemoved(SurfaceId(1)));
    let count = h.trace.len();
    h.advance_ms(100);
    assert_eq!(h.trace.len(), count);
}
#[test]
fn renderer_places_widgets_at_configured_widths_and_reports_context() {
    let mut h = Harness::new(CoreConfig::default());
    h.renderer.width("level", 120.0);
    h.renderer.width("microphone", 80.0);
    h.send(Input::Catalog(controls()));
    h.send(Input::SurfaceAdded(headless(1)));
    let level = h.hit(SurfaceId(1), "level");
    let mic = h.hit(SurfaceId(1), "microphone");
    assert_eq!(level.x, 60.0);
    assert_eq!(mic.x, 160.0);
    assert!(h.snapshot().surfaces[0].layout_generation.is_some());
    assert!(h.logic.calls.iter().any(|(_,_,call)|matches!(call,LogicCall::Handle(LogicEvent::ContextChanged(ctx)) if ctx.slot_width==400.0)));
}
#[test]
fn scenario_dsl_targets_widgets_and_checks_real_effect_commands() {
    let h = scenario()
        .logic(volume_logic())
        .catalog(controls())
        .fact("app.id", "org.mozilla.firefox")
        .tap("microphone")
        .touch_down(2, at("level"))
        .after_ms(50)
        .touch_up(2)
        .expect_effect(|effect| {
            matches!(
                effect,
                Effect::AudioSetMute {
                    target: AudioTarget::DefaultSource,
                    ..
                }
            )
        })
        .run();
    assert_eq!(h.now(), Timestamp(50));
    assert!(h.snapshot().captures.is_empty());
    assert_eq!(h.dump().replay().unwrap().core.snapshot(), h.snapshot());
}
#[test]
fn builder_produces_valid_catalog_and_invalid_catalog_is_not_silently_accepted() {
    assert!(controls().validate().is_empty());
    assert!(
        std::panic::catch_unwind(|| catalog()
            .plugin("p")
            .profile("bad", |p| {
                p.default().slot("main", &["p/missing"]);
            })
            .build())
        .is_err()
    );
}
#[test]
fn reusable_panel_fixture_routes_selection_and_automatically_acknowledges_frames() {
    let mut h = scenario()
        .logic(fixtures::controls_logic())
        .catalog(fixtures::controls_with_panel())
        .touch_down(1, at("microphone"))
        .after_ms(100)
        .run();
    assert!(h.snapshot().surfaces[0].presentation.is_some());
    let point = h.hit(SurfaceId(1), "choose");
    h.touch(SurfaceId(1), 2, TouchPhase::Down, point);
    h.touch_up(SurfaceId(1), 2);
    h.touch_up(SurfaceId(1), 1);
    assert!(h.snapshot().surfaces[0].presentation.is_none());
    assert!(
        h.effects
            .started
            .iter()
            .any(|(_, effect)| matches!(effect, Effect::AudioSetMute { .. }))
    );
    assert_eq!(h.dump().replay().unwrap().core.snapshot(), h.snapshot());
}
#[test]
fn late_layout_does_not_make_a_removed_surface_targetable() {
    let mut h = Harness::new(CoreConfig::default());
    h.renderer.delay_ms = 10;
    h.send(Input::Catalog(controls()));
    h.send(Input::SurfaceAdded(headless(1)));
    h.send(Input::SurfaceRemoved(SurfaceId(1)));
    h.advance_ms(10);
    assert!(h.snapshot().surfaces.is_empty());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.hit(SurfaceId(1), "level")))
            .is_err()
    );
}
#[test]
fn slow_logic_response_after_hard_deadline_is_not_delivered() {
    let mut logic = FakeLogic::default();
    logic.register("controls/volume", None, |_, _| LogicReaction::Delayed {
        ms: 100,
        update: LogicUpdate::default(),
    });
    let mut h = setup(logic);
    h.advance_ms(100);
    assert_eq!(h.snapshot().instances[0].status, InstanceStatus::Faulted);
    assert!(
        h.trace
            .iter()
            .all(|e| !matches!(e.input, Input::LogicUpdate { .. }))
    );
}
#[test]
fn lost_subscription_stops_the_fake_source_before_core_retries_it() {
    let mut logic = FakeLogic::default();
    logic.register("controls/volume", None, |_, call| {
        let mut u = LogicUpdate::default();
        if matches!(call, LogicCall::Init { .. }) {
            u.subscriptions = Some(vec![SubscriptionSpec {
                tag: "clock".into(),
                kind: SubscriptionKind::TimerInterval { ms: 20 },
                background: false,
            }]);
        }
        LogicReaction::Update(u)
    });
    let mut h = setup(logic);
    let sub = h.snapshot().subscriptions[0].active.unwrap();
    h.send(Input::SubscriptionLost {
        subscription: sub,
        error: EffectError::Unavailable,
    });
    let entries = h.trace.len();
    h.advance_ms(40);
    assert_eq!(h.trace.len(), entries);
    h.advance_ms(960);
    assert!(
        h.snapshot().subscriptions[0]
            .active
            .is_some_and(|new| new != sub)
    );
}
#[test]
fn ordinary_logic_error_completes_the_call_without_faulting_instance() {
    let mut logic = FakeLogic::default();
    logic.register("controls/volume", None, |_, call| {
        if matches!(call, LogicCall::Handle(LogicEvent::Action(_))) {
            LogicReaction::Error("ordinary error".into())
        } else {
            LogicReaction::Update(LogicUpdate::default())
        }
    });
    let mut h = setup(logic);
    h.tap(SurfaceId(1), "microphone");
    h.tap(SurfaceId(1), "microphone");
    assert_eq!(h.snapshot().instances[0].status, InstanceStatus::Visible);
    assert_eq!(
        h.trace
            .iter()
            .filter(|e| matches!(e.input, Input::LogicError { .. }))
            .count(),
        2
    );
}
