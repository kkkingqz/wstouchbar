use proptest::prelude::*;
use touchbar_logic_sdk::{bindings::exports::wstouchbar::logic::api as w, translation as t};
use touchbar_model::*;
use touchbar_testkit::logic_strategies::*;

proptest! {
    #[test]
    fn updates_round_trip_every_payload_variant(update in logic_update_strategy()) {
        let decoded = t::update_from(t::update_to(update.clone()));
        prop_assert!(decoded.warnings.is_empty());
        // Native translation retains even NaN payloads; Wasmtime may canonicalize them.
        prop_assert_eq!(serde_json::to_value(decoded.value).unwrap(), serde_json::to_value(update).unwrap());
    }
    #[test]
    fn events_round_trip_every_payload_variant(event in logic_event_strategy()) {
        let decoded = t::event_from(t::event_to(event.clone())).unwrap();
        prop_assert_eq!(serde_json::to_value(decoded).unwrap(), serde_json::to_value(event).unwrap());
    }
}

#[test]
fn description_round_trips_record_schema_canvas_and_action_kinds() {
    let desc = LogicDescription {
        actions: vec![(
            "action".into(),
            ActionDescription {
                kinds: vec![
                    ActionKind::Pressed,
                    ActionKind::LongPressed,
                    ActionKind::Activated,
                    ActionKind::Released,
                    ActionKind::Cancelled,
                    ActionKind::ValueChanged,
                    ActionKind::Selected,
                ],
                value: Some(ScalarKind::Number),
            },
        )],
        state: vec![
            StateField {
                name: "boolean".into(),
                kind: FieldKind::Scalar(ScalarKind::Boolean),
                persist: true,
            },
            StateField {
                name: "number".into(),
                kind: FieldKind::Scalar(ScalarKind::Number),
                persist: false,
            },
            StateField {
                name: "text".into(),
                kind: FieldKind::Scalar(ScalarKind::Text),
                persist: false,
            },
            StateField {
                name: "records".into(),
                kind: FieldKind::Records(vec![("label".into(), ScalarKind::Text)]),
                persist: false,
            },
            StateField {
                name: "canvas".into(),
                kind: FieldKind::Canvas,
                persist: false,
            },
        ],
    };
    assert_eq!(t::description_from(t::description_to(desc.clone())), desc);
}

#[test]
fn unrepresentable_items_are_dropped_with_warnings_without_losing_other_items() {
    let mut wire = t::update_to(LogicUpdate::default());
    wire.state = vec![(
        "records".into(),
        w::Value::Records(vec![
            vec![
                ("a".into(), w::Scalar::Boolean(true)),
                ("a".into(), w::Scalar::Boolean(false)),
            ],
            vec![("b".into(), w::Scalar::Boolean(true))],
        ]),
    )];
    wire.presentation = Some(w::Command::Open(w::OpenPanel {
        panel: "Invalid/Panel".into(),
        placement: w::Placement::Full,
        lifecycle: w::Lifecycle::Persistent,
    }));
    wire.requests.push(w::Request {
        tag: "bad-env".into(),
        effect: w::Effect::SpawnRun(w::SpawnSpec {
            argv: vec!["cmd".into()],
            env: vec![("A".into(), "first".into()), ("A".into(), "second".into())],
            stdin: None,
            max_output: 100,
        }),
        lifetime: None,
        coalesce: None,
        timeout_ms: None,
    });
    wire.cancel = vec!["keep".into()];
    let translated = t::update_from(wire);
    assert_eq!(translated.warnings.len(), 3);
    assert_eq!(translated.value.cancel, ["keep"]);
    assert_eq!(
        translated.value.state,
        vec![(
            "records".into(),
            StateValue::Records(vec![[("b".into(), Scalar::Boolean(true))].into()])
        )]
    );
    assert!(translated.value.requests.is_empty());
    assert!(translated.value.presentation.is_none());
}
