use touchbar_core::Input;
use touchbar_model::*;
use touchbar_testkit::*;

fn controls(config: &str) -> ModuleTester {
    let mut m = ModuleTester::load(touchbar_wasm_fixtures::CONTROLS);
    m.init(config, vec![]);
    m
}
fn done(tag: String, result: EffectOutcome) -> LogicEvent {
    LogicEvent::EffectDone { tag, result }
}
fn fact(value: Option<f64>) -> LogicEvent {
    LogicEvent::SubscriptionData {
        tag: "brightness".into(),
        data: SubscriptionData::Fact(value.map(FactValue::Number)),
    }
}
#[test]
fn controls_configurations_declare_foreground_subscriptions_and_reject_unknown() {
    for (config, tags) in [
        ("volume", vec!["sink", "source"]),
        ("microphone", vec!["source"]),
        ("brightness", vec!["brightness"]),
    ] {
        let mut m = controls(config);
        m.expect_subscriptions(|s| {
            s.iter().map(|s| s.tag.as_str()).collect::<Vec<_>>() == tags
                && s.iter().all(|s| !s.background)
        });
        assert!(m.description.state.iter().all(|s| !s.persist));
        m.expect_state("available", false);
    }
    let m = controls("unknown");
    assert!(matches!(m.input, Some(Input::LogicError { .. })));
}
#[test]
fn volume_tracks_external_state_and_clamps_only_slider() {
    let mut m = controls("volume");
    m.handle(audio_data("sink", 1.25, false))
        .expect_state("volume", 1.0)
        .expect_state("volume_text", "125%")
        .expect_state("volume_label", "VOL 125%");
    m.handle(audio_data("sink", 0.5, true))
        .expect_state("volume_text", "MUTED")
        .expect_state("volume_icon", "muted")
        .expect_state("volume_selected", true);
}
#[test]
fn volume_drag_is_optimistic_unmutes_once_and_restores_latest_subscription() {
    let mut m = controls("volume");
    m.handle(audio_data("sink", 0.2, true));
    m.handle(action("volume-level", ActionKind::Pressed, 0.2));
    m.handle(action("volume-level", ActionKind::ValueChanged, 0.8))
        .expect_state("volume", 0.8)
        .expect_state("volume_text", "80%");
    assert!(matches!(
        m.update.requests[0].effect,
        Effect::AudioSetMute {
            target: AudioTarget::DefaultSink,
            value: MuteValue::Off
        }
    ));
    m.expect_request(|r| {
        r.coalesce.as_deref() == Some("volume") && r.policy().lifetime == Lifetime::Committed
    });
    m.handle(audio_data("sink", 0.3, true))
        .expect_state("volume", 0.8);
    m.handle(action("volume-level", ActionKind::ValueChanged, 0.9));
    assert_eq!(m.update.requests.len(), 1);
    m.handle(action("volume-level", ActionKind::Cancelled, 0.9))
        .expect_state("volume", 0.3)
        .expect_state("volume_text", "MUTED");
    assert!(m.update.cancel.is_empty());
    m.handle(action("volume-level", ActionKind::Pressed, 0.3))
        .handle(action("volume-level", ActionKind::ValueChanged, 0.7));
    m.handle(audio_data("sink", 0.6, false))
        .expect_state("volume", 0.7);
    m.handle(action("volume-level", ActionKind::Released, 0.7))
        .expect_state("volume", 0.6);
}
#[test]
fn microphone_results_confirm_or_revert_latest_baseline_and_ignore_old_toggle() {
    let mut m = controls("microphone");
    m.handle(audio_data("source", 1.0, false));
    m.handle(action("microphone", ActionKind::Activated, 0.0))
        .expect_state("microphone_muted", true);
    let first = m.update.requests[0].tag.clone();
    m.handle(audio_data("source", 1.0, true));
    m.handle(action("microphone", ActionKind::Activated, 0.0))
        .expect_state("microphone_muted", false);
    let second = m.update.requests[0].tag.clone();
    assert_ne!(first, second);
    m.handle(done(first, Ok(EffectResult::Mute(true))))
        .expect_state("microphone_muted", false);
    m.handle(done(second, Err(EffectError::Unavailable)))
        .expect_state("microphone_muted", true);
    m.handle(action("microphone", ActionKind::Activated, 0.0));
    let tag = m.update.requests[0].tag.clone();
    m.handle(done(tag, Ok(EffectResult::Mute(false))))
        .expect_state("microphone_muted", false);
}
#[test]
fn brightness_drag_and_unavailability_recover() {
    let mut m = controls("brightness");
    m.handle(fact(Some(0.3)))
        .expect_state("brightness_text", "30%")
        .expect_state("available", true);
    m.handle(action("brightness-level", ActionKind::Pressed, 0.3))
        .handle(action("brightness-level", ActionKind::ValueChanged, 0.8));
    m.expect_request(|r| {
        r.coalesce.as_deref() == Some("brightness")
            && matches!(r.effect, Effect::DesktopBrightness { level } if level == 0.8)
    });
    let tag = m.update.requests[0].tag.clone();
    m.handle(fact(Some(0.4))).expect_state("brightness", 0.8);
    m.handle(action("brightness-level", ActionKind::Released, 0.8))
        .expect_state("brightness", 0.4);
    m.handle(done(tag, Err(EffectError::Unsupported)))
        .expect_state("available", false)
        .expect_state("brightness_text", "–");
    m.handle(fact(Some(0.6)))
        .expect_state("available", true)
        .expect_state("brightness_label", "LIGHT 60%");
    m.handle(fact(None)).expect_state("available", false);
}
#[test]
fn audio_loss_disables_controls_and_recovery_enables_them() {
    let mut m = controls("volume");
    m.handle(audio_data("sink", 0.5, false));
    m.handle(LogicEvent::SubscriptionLost {
        tag: "sink".into(),
        error: EffectError::Unavailable,
    })
    .expect_state("available", false)
    .expect_state("volume_text", "–");
    m.handle(action("volume-level", ActionKind::ValueChanged, 0.5));
    assert!(m.update.requests.is_empty());
    m.handle(audio_data("sink", 0.8, false))
        .expect_state("available", true);
    m.handle(LogicEvent::SubscriptionData {
        tag: "source".into(),
        data: SubscriptionData::Audio(AudioState {
            available: false,
            volume: 0.0,
            muted: false,
            name: String::new(),
        }),
    })
    .expect_state("microphone_available", false)
    .expect_state("microphone_text", "–");
}

#[test]
fn slider_up_activation_restores_latest_system_value() {
    let mut m = controls("volume");
    m.handle(audio_data("sink", 0.2, false));
    m.handle(action("volume-level", ActionKind::Pressed, 0.2))
        .handle(action("volume-level", ActionKind::ValueChanged, 0.8));
    m.handle(audio_data("sink", 0.4, false))
        .expect_state("volume", 0.8);
    m.handle(action("volume-level", ActionKind::Activated, 0.8))
        .expect_state("volume", 0.4);
    assert!(m.update.requests.is_empty());
}
