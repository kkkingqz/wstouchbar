use touchbar_core::*;
use touchbar_model::*;
use touchbar_testkit::*;
#[test]
fn module_tester_executes_stateful_guest_and_uses_call_timestamp() {
    let mut m = ModuleTester::load(touchbar_wasm_fixtures::PERSIST);
    m.init(
        "",
        vec![("value".into(), StateValue::Scalar(Scalar::Number(7.0)))],
    )
    .expect_state("value", Scalar::Number(7.0));
    m.handle(action("change", ActionKind::Activated, 9.0))
        .expect_state("value", Scalar::Number(9.0));
    m.init("", vec![])
        .expect_state("value", Scalar::Number(0.0));
    let mut echo = ModuleTester::load(touchbar_wasm_fixtures::ECHO);
    echo.init("", vec![]).advance_ms(700);
    echo.handle(audio_data("sink", 0.4, false));
    let StateValue::Scalar(Scalar::Text(last)) = &echo.update.state[0].1 else {
        panic!("text")
    };
    let json: serde_json::Value = serde_json::from_str(last).unwrap();
    assert_eq!(json["context"]["now_ms"], 700);
}
#[test]
fn module_tester_exposes_errors_and_real_faults() {
    let mut m = ModuleTester::load(touchbar_wasm_fixtures::ERR);
    m.init("init", vec![]);
    assert!(matches!(&m.input,Some(Input::LogicError{message,..}) if message.contains("init")));
    let mut m = ModuleTester::load(touchbar_wasm_fixtures::HANG);
    m.init("", vec![])
        .handle(action("hang", ActionKind::Activated, 0.0));
    assert!(matches!(&m.input,Some(Input::LogicFault{reason,..}) if reason.contains("timeout")));
}
#[test]
fn request_and_subscription_assertions_check_real_guest_output() {
    let mut m = ModuleTester::load(touchbar_wasm_fixtures::ECHO);
    let update = LogicUpdate {
        requests: vec![EffectRequest {
            tag: "later".into(),
            effect: Effect::TimerAfter { ms: 5 },
            lifetime: Some(Lifetime::Committed),
            coalesce: None,
            timeout_ms: None,
        }],
        subscriptions: Some(vec![SubscriptionSpec {
            tag: "sink".into(),
            kind: SubscriptionKind::AudioWatch {
                target: AudioTarget::DefaultSink,
            },
            background: false,
        }]),
        ..Default::default()
    };
    m.init(&serde_json::to_string(&update).unwrap(), vec![])
        .expect_request(|r| r.tag == "later")
        .expect_subscriptions(|s| s.len() == 1);
}
