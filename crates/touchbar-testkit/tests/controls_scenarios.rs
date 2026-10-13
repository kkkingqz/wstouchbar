#[allow(dead_code)] // This integration binary uses only the shared fixture helpers.
mod common;
use touchbar_core::*;
use touchbar_model::*;
use touchbar_testkit::*;
fn setup(surfaces: u64, effects: FakeEffects, trap: bool) -> Harness<WasmLogic> {
    let (plugin, host) = common::controls_host();
    let mut host = host.unwrap();
    let mut c = common::controls_catalog(&plugin, 1, false, false);
    if trap {
        host.register("trap", "test-1", touchbar_wasm_fixtures::TRAP)
            .unwrap();
        c.plugins.push(PluginSpec {
            id: "trap".parse().unwrap(),
            revision: "test-1".into(),
            enabled: true,
            has_logic: true,
            requires: vec![],
        });
        c.sets
            .iter_mut()
            .find(|s| s.reference.id() == "volume")
            .unwrap()
            .logic = Some("trap".parse().unwrap());
    }
    let mut h = Harness::with_logic(CoreConfig::default(), host);
    h.enable_invariant_checks();
    h.effects = effects;
    h.send(Input::Catalog(c));
    for id in 1..=surfaces {
        let mut info = headless(id);
        info.width_px = 1200;
        info.height_px = 60;
        info.caps.modes = true;
        h.send(Input::SurfaceAdded(info));
        h.send(Input::Mode {
            surface: SurfaceId(id),
            mode: Mode::Display,
        });
    }
    common::seed_subscriptions(&mut h);
    h
}
fn ready() -> Harness<WasmLogic> {
    setup(1, FakeEffects::default(), false)
}
fn checked(mut h: Harness<WasmLogic>) {
    assert!(h.invariant_error.is_none(), "{:?}", h.invariant_error);
    for s in h.snapshot().surfaces {
        h.send(Input::SurfaceRemoved(s.info.id));
    }
    h.advance_ms(10000);
    h.finish_invariant_checks().unwrap();
    assert!(h.host_errors.is_empty());
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
fn state(h: &Harness<WasmLogic>, surface: u64, set: &str, name: &str) -> Scalar {
    let snapshot = h.snapshot();
    let i = snapshot
        .instances
        .iter()
        .find(|i| i.surface == SurfaceId(surface) && i.set.id() == set)
        .unwrap();
    let StateValue::Scalar(value) = &i.state.iter().find(|(n, _)| n == name).unwrap().1 else {
        panic!("scalar")
    };
    value.clone()
}
fn audio(
    h: &mut Harness<WasmLogic>,
    surface: u64,
    set: &str,
    tag: &str,
    level: f64,
    muted: bool,
    available: bool,
) {
    h.subscription_data(
        SurfaceId(surface),
        &format!("controls/{set}"),
        tag,
        SubscriptionData::Audio(AudioState {
            volume: level,
            muted,
            available,
            name: "test".into(),
        }),
    );
}
fn brightness(h: &mut Harness<WasmLogic>, level: f64) {
    h.subscription_data(
        SurfaceId(1),
        "controls/brightness",
        "brightness",
        SubscriptionData::Fact(Some(FactValue::Number(level))),
    );
}
fn down(h: &mut Harness<WasmLogic>, widget: &str, fraction: f32) -> Point {
    let mut point = h.hit(SurfaceId(1), widget);
    point.x += (fraction - 0.5) * 80.0;
    h.touch(SurfaceId(1), 1, TouchPhase::Down, point);
    point
}
fn fn_key(h: &mut Harness<WasmLogic>, pressed: bool) {
    h.send(Input::Key {
        source: KeySource::Surface(SurfaceId(1)),
        code: WatchKey::Fn,
        phase: if pressed {
            KeyPhase::Pressed
        } else {
            KeyPhase::Released
        },
    });
}
#[test]
fn controls_describe_matches_sets() {
    let h = ready();
    assert_eq!(h.snapshot().instances.len(), 3);
    assert!(
        h.snapshot()
            .instances
            .iter()
            .all(|i| i.status == InstanceStatus::Visible)
    );
    assert_eq!(
        h.snapshot()
            .subscriptions
            .iter()
            .filter(|s| s.active.is_some())
            .count(),
        4
    );
    checked(h);
}
#[test]
fn volume_follows_external_changes() {
    let mut h = ready();
    audio(&mut h, 1, "volume", "sink", 1.25, false, true);
    assert_eq!(state(&h, 1, "volume", "volume"), Scalar::Number(1.0));
    assert_eq!(
        state(&h, 1, "volume", "volume_text"),
        Scalar::Text("125%".into())
    );
    down(&mut h, "volume-level", 0.75);
    audio(&mut h, 1, "volume", "sink", 0.2, true, true);
    assert_eq!(state(&h, 1, "volume", "volume"), Scalar::Number(0.75));
    h.touch_up(SurfaceId(1), 1);
    assert_eq!(state(&h, 1, "volume", "volume"), Scalar::Number(0.2));
    assert_eq!(
        state(&h, 1, "volume", "volume_icon"),
        Scalar::Text("muted".into())
    );
    checked(h);
}
#[test]
fn volume_drag_coalesces() {
    let mut h = setup(
        1,
        FakeEffects::with_handler(|_| (1000, Ok(EffectResult::None))),
        false,
    );
    let start = down(&mut h, "volume-level", 0.2);
    for n in 1..50 {
        h.advance_ms(20);
        h.touch(
            SurfaceId(1),
            1,
            TouchPhase::Move,
            Point {
                x: start.x + 56.0 * n as f32 / 49.0,
                y: start.y,
            },
        );
    }
    h.touch_up(SurfaceId(1), 1);
    h.advance_ms(2020);
    let writes: Vec<_> = h
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
    assert_eq!(h.logic.calls.iter().filter(|(_, _, c)| matches!(c, LogicCall::Handle(LogicEvent::Action(a)) if a.action == "volume-level" && a.kind == ActionKind::ValueChanged)).count(), 50);
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert!((writes[1] - 0.9).abs() < 0.00001);
    checked(h);
}
#[test]
fn volume_drag_unmutes() {
    let mut h = ready();
    audio(&mut h, 1, "volume", "sink", 0.4, true, true);
    h.drag(SurfaceId(1), "volume-level", 0.4, 0.9, 500, 20);
    assert!(matches!(
        h.effects.started[0].1,
        Effect::AudioSetMute {
            target: AudioTarget::DefaultSink,
            value: MuteValue::Off
        }
    ));
    assert!(matches!(
        h.effects.started[1].1,
        Effect::AudioSetVolume { .. }
    ));
    assert_eq!(
        h.effects
            .started
            .iter()
            .filter(|(_, e)| matches!(
                e,
                Effect::AudioSetMute {
                    target: AudioTarget::DefaultSink,
                    ..
                }
            ))
            .count(),
        1
    );
    checked(h);
}
#[test]
fn volume_cancel_restores() {
    let mut h = ready();
    down(&mut h, "volume-level", 0.8);
    audio(&mut h, 1, "volume", "sink", 0.3, false, true);
    fn_key(&mut h, true);
    assert_eq!(state(&h, 1, "volume", "volume"), Scalar::Number(0.3));
    assert!(h.logic.calls.iter().any(|(_, _, call)| matches!(call, LogicCall::Handle(LogicEvent::Action(e)) if e.kind == ActionKind::Cancelled && e.cancel_reason == Some(CancelReason::Layer))));
    fn_key(&mut h, false);
    checked(h);
}
#[test]
fn volume_panel_tap_and_hold() {
    let mut h = ready();
    h.tap(SurfaceId(1), "volume-panel");
    assert_eq!(
        h.snapshot().surfaces[0]
            .presentation
            .as_ref()
            .unwrap()
            .lifecycle,
        PresentationLifecycle::Persistent
    );
    // Tap the uncovered brightness control to dismiss before holding the anchor.
    h.tap(SurfaceId(1), "brightness-panel");
    // Outside selection may open another panel; a mode transition closes it.
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Native,
    });
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Display,
    });
    let point = h.hit(SurfaceId(1), "volume-panel");
    h.touch(SurfaceId(1), 1, TouchPhase::Down, point);
    h.advance_ms(419);
    assert!(h.snapshot().surfaces[0].presentation.is_none());
    h.advance_ms(1);
    assert_eq!(
        h.snapshot().surfaces[0]
            .presentation
            .as_ref()
            .unwrap()
            .lifecycle,
        PresentationLifecycle::Transient
    );
    h.touch_up(SurfaceId(1), 1);
    assert!(h.snapshot().surfaces[0].presentation.is_none());
    checked(h);
}
#[test]
fn mic_toggle_in_panel_survives_close() {
    let mut h = setup(
        1,
        FakeEffects::with_handler(|e| {
            (
                500,
                if matches!(e, Effect::AudioSetMute { .. }) {
                    Ok(EffectResult::Mute(true))
                } else {
                    Ok(EffectResult::None)
                },
            )
        }),
        false,
    );
    h.tap(SurfaceId(1), "volume-panel");
    h.tap(SurfaceId(1), "volume-panel-microphone");
    assert!(h.snapshot().surfaces[0].presentation.is_none());
    assert_eq!(h.effects.started.len(), 1);
    assert!(h.effects.cancelled.is_empty());
    assert!(h.snapshot().requests[0].running);
    h.advance_ms(500);
    assert_eq!(
        state(&h, 1, "volume", "microphone_muted"),
        Scalar::Boolean(true)
    );
    checked(h);
}
#[test]
fn mic_toggle_failure_reverts() {
    let mut h = setup(
        1,
        FakeEffects::with_handler(|_| (100, Err(EffectError::Failed("denied".into())))),
        false,
    );
    h.tap(SurfaceId(1), "microphone-toggle");
    assert_eq!(
        state(&h, 1, "microphone", "microphone_muted"),
        Scalar::Boolean(true)
    );
    h.advance_ms(100);
    assert_eq!(
        state(&h, 1, "microphone", "microphone_muted"),
        Scalar::Boolean(false)
    );
    checked(h);
}
#[test]
fn hidden_controls_hold_no_watchers() {
    let (plugin, host) = common::controls_host();
    let mut c = common::controls_catalog(&plugin, 1, false, false);
    c.profiles[0].slots.truncate(1);
    let mut h = Harness::with_logic(CoreConfig::default(), host.unwrap());
    h.enable_invariant_checks();
    h.send(Input::Catalog(c));
    let mut info = headless(1);
    info.caps.modes = true;
    h.send(Input::SurfaceAdded(info));
    assert!(
        h.snapshot()
            .subscriptions
            .iter()
            .all(|s| s.active.is_none()),
        "never-shown controls must hold no active watcher"
    );
    assert!(
        h.trace
            .iter()
            .flat_map(|t| &t.outputs)
            .all(|o| !matches!(o, Output::Subscribe { .. }))
    );
    assert!(
        h.logic
            .calls
            .iter()
            .all(|(_, _, c)| !matches!(c, LogicCall::Handle(LogicEvent::Shown)))
    );
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Display,
    });
    assert_eq!(h.snapshot().instances.len(), 1);
    assert_eq!(h.snapshot().subscriptions.len(), 2);
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Native,
    });
    assert!(
        h.snapshot()
            .subscriptions
            .iter()
            .all(|s| s.active.is_none())
    );
    h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Display,
    });
    assert_eq!(
        h.snapshot()
            .subscriptions
            .iter()
            .filter(|s| s.active.is_some())
            .count(),
        2
    );
    checked(h);
}
#[test]
fn audio_unavailable_disables() {
    let mut h = ready();
    let sub = h
        .snapshot()
        .subscriptions
        .iter()
        .find(|s| s.spec.tag == "sink")
        .and_then(|s| s.active)
        .unwrap();
    h.send(Input::SubscriptionLost {
        subscription: sub,
        error: EffectError::Unavailable,
    });
    assert_eq!(state(&h, 1, "volume", "available"), Scalar::Boolean(false));
    h.tap(SurfaceId(1), "volume-panel");
    assert!(h.snapshot().surfaces[0].presentation.is_none());
    h.advance_ms(1000);
    audio(&mut h, 1, "volume", "sink", 0.5, false, true);
    assert_eq!(state(&h, 1, "volume", "available"), Scalar::Boolean(true));
    audio(&mut h, 1, "microphone", "source", 0.5, false, false);
    h.tap(SurfaceId(1), "microphone-toggle");
    assert!(h.effects.started.is_empty());
    checked(h);
}
#[test]
fn brightness_follows_fact() {
    let mut h = ready();
    brightness(&mut h, 0.7);
    assert_eq!(
        state(&h, 1, "brightness", "brightness"),
        Scalar::Number(0.7)
    );
    h.drag(SurfaceId(1), "brightness-level", 0.7, 0.9, 200, 10);
    assert!(h.effects.started.iter().any(
        |(_, e)| matches!(e, Effect::DesktopBrightness { level } if (*level - 0.9).abs() < 0.00001)
    ));
    checked(h);
}
#[test]
fn brightness_unsupported() {
    let mut h = setup(
        1,
        FakeEffects::with_handler(|_| (0, Err(EffectError::Unsupported))),
        false,
    );
    h.drag(SurfaceId(1), "brightness-level", 0.5, 0.8, 200, 10);
    assert_eq!(
        state(&h, 1, "brightness", "available"),
        Scalar::Boolean(false)
    );
    assert_eq!(
        state(&h, 1, "brightness", "brightness_text"),
        Scalar::Text("–".into())
    );
    assert_eq!(
        h.snapshot()
            .instances
            .iter()
            .find(|i| i.set.id() == "brightness")
            .unwrap()
            .status,
        InstanceStatus::Visible
    );
    checked(h);
}
#[test]
fn controls_on_two_surfaces() {
    let mut h = setup(2, FakeEffects::default(), false);
    audio(&mut h, 2, "volume", "sink", 0.9, false, true);
    assert_eq!(state(&h, 1, "volume", "volume"), Scalar::Number(0.4));
    assert_eq!(state(&h, 2, "volume", "volume"), Scalar::Number(0.9));
    h.tap(SurfaceId(2), "microphone-toggle");
    assert!(
        h.logic
            .calls
            .iter()
            .any(|(_, _, c)| matches!(c, LogicCall::Handle(LogicEvent::Action(_))))
    );
    checked(h);
}
#[test]
fn controls_fault_is_isolated() {
    let mut h = setup(1, FakeEffects::default(), true);
    assert!(
        h.snapshot()
            .instances
            .iter()
            .all(|i| i.status == InstanceStatus::Visible),
        "{:?}",
        h.snapshot().instances
    );
    let before = state(&h, 1, "brightness", "brightness");
    down(&mut h, "volume-level", 0.5);
    assert_eq!(
        h.snapshot()
            .instances
            .iter()
            .find(|i| i.set.id() == "volume")
            .unwrap()
            .status,
        InstanceStatus::Faulted
    );
    assert_eq!(state(&h, 1, "brightness", "brightness"), before);
    assert!(
        h.trace
            .iter()
            .any(|t| matches!(t.input, Input::LogicFault { .. }))
    );
    brightness(&mut h, 0.9);
    assert_eq!(
        state(&h, 1, "brightness", "brightness"),
        Scalar::Number(0.9)
    );
    checked(h);
}
