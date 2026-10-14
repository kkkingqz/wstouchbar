use touchbar_logic_sdk::{Description, Request, Update};
use touchbar_model::*;

#[test]
fn builders_preserve_description_state_and_effect_policy() {
    let description = Description::new()
        .action(
            "level",
            vec![ActionKind::Pressed, ActionKind::ValueChanged],
            Some(ScalarKind::Number),
        )
        .field("value", FieldKind::Scalar(ScalarKind::Number), false)
        .build();
    assert_eq!(description.actions.len(), 1);
    assert_eq!(description.actions[0].0, "level");
    assert_eq!(
        description.actions[0].1.kinds,
        [ActionKind::Pressed, ActionKind::ValueChanged]
    );
    assert_eq!(description.state[0].name, "value");
    let update = Update::new()
        .set("value", 0.7)
        .set("muted", true)
        .set("label", "70%")
        .request(
            Request::new("write", Effect::DesktopBrightness { level: 0.7 })
                .coalesce("brightness")
                .committed()
                .timeout(80),
        )
        .cancel("old")
        .subscriptions(vec![])
        .presentation(PresentationCommand::Close)
        .build();
    assert_eq!(
        update.state,
        vec![
            ("value".into(), StateValue::Scalar(Scalar::Number(0.7))),
            ("muted".into(), StateValue::Scalar(Scalar::Boolean(true))),
            (
                "label".into(),
                StateValue::Scalar(Scalar::Text("70%".into()))
            )
        ]
    );
    assert_eq!(update.cancel, ["old"]);
    assert_eq!(update.subscriptions, Some(vec![]));
    assert_eq!(update.presentation, Some(PresentationCommand::Close));
    assert_eq!(
        update.requests[0],
        EffectRequest {
            tag: "write".into(),
            effect: Effect::DesktopBrightness { level: 0.7 },
            lifetime: Some(Lifetime::Committed),
            coalesce: Some("brightness".into()),
            timeout_ms: Some(80)
        }
    );
}

#[test]
fn unused_builder_preserves_absent_subscription_update_and_default_policy() {
    assert_eq!(Update::new().build(), LogicUpdate::default());
    let request = Request::new("timer", Effect::TimerAfter { ms: 10 }).build();
    assert_eq!(request.lifetime, None);
    assert_eq!(request.coalesce, None);
    assert_eq!(request.timeout_ms, None);
}

#[cfg(debug_assertions)]
#[test]
fn debug_builder_reports_oversized_updates_immediately() {
    let result =
        std::panic::catch_unwind(|| Update::new().set("large", "x".repeat(300_000)).build());
    assert!(result.is_err());
}
