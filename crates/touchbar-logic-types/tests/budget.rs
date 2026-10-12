use proptest::prelude::*;
use touchbar_logic_types::{LogicLimits, check_budget, wire_size};
use touchbar_model::*;

#[test]
fn exact_budget_boundary_is_accepted_and_one_byte_more_is_rejected() {
    let update = LogicUpdate {
        state: vec![("a".into(), StateValue::Scalar(Scalar::Text("é".into())))],
        ..Default::default()
    };
    // Header 64 + field 16 + key (8+1) + scalar tag 8 + text (8+2).
    assert_eq!(wire_size(&update), 107);
    let limits = LogicLimits {
        max_update_bytes: 107,
        ..Default::default()
    };
    assert!(check_budget(&update, &limits).is_ok());
    assert!(
        check_budget(
            &update,
            &LogicLimits {
                max_update_bytes: 106,
                ..limits
            }
        )
        .is_err()
    );
}

#[test]
fn global_budget_does_not_replace_element_validation() {
    let update = LogicUpdate {
        requests: vec![EffectRequest {
            tag: "".into(),
            effect: Effect::DesktopBrightness { level: f64::NAN },
            lifetime: None,
            coalesce: None,
            timeout_ms: Some(0),
        }],
        ..Default::default()
    };
    assert!(check_budget(&update, &LogicLimits::default()).is_ok());
    assert!(update.requests[0].validate().is_err());
}

#[test]
fn strings_in_nested_effects_records_and_canvas_count_against_budget() {
    let empty = LogicUpdate::default();
    let mut update = empty.clone();
    update.state.push((
        "records".into(),
        StateValue::Records(vec![
            [("text".into(), Scalar::Text("x".repeat(300_000)))].into(),
        ]),
    ));
    assert!(check_budget(&update, &LogicLimits::default()).is_err());
    update.state = vec![(
        "canvas".into(),
        StateValue::Canvas(vec![CanvasCommand::Text {
            rect: Rect::default(),
            text: "x".repeat(300_000),
            size: 10.0,
            paint: CanvasPaint {
                color: CanvasColor::Role(ColorRole::Foreground),
                opacity: 1.0,
            },
            align: TextAlign::Leading,
        }]),
    )];
    assert!(check_budget(&update, &LogicLimits::default()).is_err());
    update.state.clear();
    update.subscriptions = Some(vec![SubscriptionSpec {
        tag: "lines".into(),
        kind: SubscriptionKind::SpawnLines(SpawnSpec {
            argv: vec!["cmd".into()],
            env: [("data".into(), "x".repeat(300_000))].into(),
            stdin: None,
            max_output: 1024,
        }),
        background: false,
    }]);
    assert!(check_budget(&update, &LogicLimits::default()).is_err());
    update.subscriptions = None;
    update.requests.push(EffectRequest {
        tag: "dbus".into(),
        effect: Effect::DbusCall(DbusCall {
            bus: Bus::Session,
            destination: "app".into(),
            path: "/".into(),
            interface: "app".into(),
            method: "call".into(),
            args: DbusValue {
                signature: "s".into(),
                json: "x".repeat(300_000),
            },
        }),
        lifetime: None,
        coalesce: None,
        timeout_ms: None,
    });
    assert!(check_budget(&update, &LogicLimits::default()).is_err());
    assert!(wire_size(&update) > wire_size(&empty));
}

proptest! {
    #[test]
    fn adding_elements_never_decreases_estimated_size(text in ".{0,256}", count in 0usize..40) {
        let mut update = LogicUpdate::default();
        let mut previous = wire_size(&update);
        for i in 0..count {
            update.state.push((format!("field-{i}"), StateValue::Scalar(Scalar::Text(text.clone()))));
            let current = wire_size(&update);
            prop_assert!(current > previous);
            previous = current;
        }
    }
}
