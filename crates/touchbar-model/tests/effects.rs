use touchbar_model::*;

#[test]
fn requests_use_kind_defaults_without_overwriting_explicit_policy() {
    let request = EffectRequest {
        tag: "volume".into(),
        effect: Effect::AudioSetVolume {
            target: AudioTarget::DefaultSink,
            level: 1.0,
        },
        lifetime: None,
        coalesce: Some("volume".into()),
        timeout_ms: None,
    };
    assert_eq!(
        request.policy(),
        EffectPolicy {
            lifetime: Lifetime::Committed,
            timeout_ms: Some(1000),
            coalesce: Some("volume".into())
        }
    );
    let request = EffectRequest {
        effect: Effect::SpawnRun(SpawnSpec::new(vec!["true".into()])),
        lifetime: Some(Lifetime::Committed),
        timeout_ms: Some(8000),
        ..request
    };
    assert_eq!(request.policy().lifetime, Lifetime::Committed);
    assert_eq!(request.policy().timeout_ms, Some(8000));
    assert_eq!(Effect::MediaNext.defaults().timeout_ms, Some(2000));
    assert_eq!(
        Effect::SpawnRun(SpawnSpec::new(vec!["true".into()]))
            .defaults()
            .lifetime,
        Lifetime::UntilHidden
    );
    assert_eq!(Effect::TimerAfter { ms: 42 }.defaults().timeout_ms, None);
}

#[test]
fn effect_validation_rejects_out_of_range_values() {
    for level in [-0.1, 1.51, f64::NAN, f64::INFINITY] {
        assert!(
            Effect::AudioSetVolume {
                target: AudioTarget::DefaultSink,
                level
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        Effect::AudioSetVolume {
            target: AudioTarget::DefaultSource,
            level: 1.5
        }
        .validate()
        .is_ok()
    );
    assert!(Effect::DesktopBrightness { level: 1.1 }.validate().is_err());
    assert!(Effect::SpawnRun(SpawnSpec::new(vec![])).validate().is_err());
    assert!(
        Effect::SpawnRun(SpawnSpec::new(vec!["bad\0arg".into()]))
            .validate()
            .is_err()
    );
    assert!(
        SubscriptionKind::TimerInterval { ms: 15 }
            .validate()
            .is_err()
    );
    assert!(
        SubscriptionKind::TimerInterval { ms: 16 }
            .validate()
            .is_ok()
    );
}

#[test]
fn init_and_event_calls_round_trip_with_state_and_requests() {
    let call = LogicCall::Init {
        config: "microphone".into(),
        persisted: vec![("muted".into(), StateValue::Scalar(Scalar::Boolean(true)))],
    };
    assert_eq!(
        serde_json::from_str::<LogicCall>(&serde_json::to_string(&call).unwrap()).unwrap(),
        call
    );
    let update = LogicUpdate {
        state: vec![("level".into(), StateValue::Scalar(Scalar::Number(0.5)))],
        requests: vec![EffectRequest {
            tag: "mute".into(),
            effect: Effect::AudioSetMute {
                target: AudioTarget::DefaultSource,
                value: MuteValue::Toggle,
            },
            lifetime: Some(Lifetime::Committed),
            coalesce: None,
            timeout_ms: None,
        }],
        subscriptions: Some(vec![SubscriptionSpec {
            tag: "source".into(),
            kind: SubscriptionKind::AudioWatch {
                target: AudioTarget::DefaultSource,
            },
            background: false,
        }]),
        ..LogicUpdate::default()
    };
    assert_eq!(
        serde_json::from_str::<LogicUpdate>(&serde_json::to_string(&update).unwrap()).unwrap(),
        update
    );
    let completion = LogicCall::Handle(LogicEvent::EffectDone {
        tag: "mute".into(),
        result: Ok(EffectResult::Mute(true)),
    });
    assert_eq!(
        serde_json::from_str::<LogicCall>(&serde_json::to_string(&completion).unwrap()).unwrap(),
        completion
    );
}

#[test]
fn state_schema_checks_types_and_finite_values() {
    let number = FieldKind::Scalar(ScalarKind::Number);
    assert!(number.accepts(&StateValue::Scalar(Scalar::Number(0.5))));
    assert!(!number.accepts(&StateValue::Scalar(Scalar::Boolean(true))));
    assert!(!number.accepts(&StateValue::Scalar(Scalar::Number(f64::NAN))));
    let records = FieldKind::Records(vec![("key".into(), ScalarKind::Text)]);
    assert!(records.accepts(&StateValue::Records(vec![
        std::collections::BTreeMap::from([("key".into(), Scalar::Text("one".into()))])
    ])));
    assert!(!records.accepts(&StateValue::Records(
        vec![std::collections::BTreeMap::new()]
    )));
}
