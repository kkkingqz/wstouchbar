use proptest::prelude::*;
use touchbar_core::*;
use touchbar_model::*;

#[test]
fn recorder_starts_with_initial_checkpoint_and_replays_early_inputs() {
    let mut c = Core::new(CoreConfig {
        persist_delay_ms: 37,
        ..CoreConfig::default()
    });
    let mut recorder = Recorder::new(&c);
    assert!(recorder.dump().inputs.is_empty());
    for n in 0..50 {
        recorder.handle(
            &mut c,
            Timestamp(n),
            Input::Fact {
                name: "counter".into(),
                value: Some(FactValue::Number(n as f64)),
            },
        );
    }
    let dump = ReplayDump::from_json(&recorder.dump().to_json().unwrap()).unwrap();
    assert_eq!(dump.checkpoint.config().persist_delay_ms, 37);
    assert_eq!(dump.inputs.len(), 50);
    let replay = dump.replay().unwrap();
    assert_eq!(replay.core.snapshot(), c.snapshot());
    assert_eq!(replay.trace, dump.inputs);
}
#[test]
fn recorder_retains_two_checkpoints_and_between_two_and_four_thousand_inputs() {
    let mut c = Core::new(CoreConfig::default());
    let mut recorder = Recorder::new(&c);
    for n in 0..8500 {
        recorder.handle(
            &mut c,
            Timestamp(n),
            Input::Fact {
                name: "counter".into(),
                value: Some(FactValue::Number(n as f64)),
            },
        );
        let count = n + 1;
        if [1999, 2000, 3999, 4000, 6500, 8500].contains(&count) {
            let dump = recorder.dump();
            let expected = if count < 4000 {
                count
            } else {
                2000 + count % 2000
            };
            assert_eq!(dump.inputs.len() as u64, expected);
            assert!(recorder.checkpoint_count() <= 2);
            assert_eq!(dump.replay().unwrap().core.snapshot(), c.snapshot());
        }
    }
}
#[test]
fn unsupported_checkpoint_version_is_reported_before_reading_its_old_payload() {
    let c = Core::new(CoreConfig::default());
    let dump = Recorder::new(&c).dump();
    let mut value = serde_json::to_value(dump).unwrap();
    value["checkpoint"]["format_version"] = serde_json::json!(0);
    value["checkpoint"]["core"] = serde_json::json!({"obsolete":"shape"});
    let error = ReplayDump::from_json(&value.to_string()).unwrap_err();
    assert!(error.to_string().contains("core state version 0"));
}
#[test]
fn replay_reports_the_first_changed_output() {
    let mut c = Core::new(CoreConfig::default());
    let mut recorder = Recorder::new(&c);
    recorder.handle(&mut c, Timestamp(0), Input::Locale("uk_UA".into()));
    let mut dump = recorder.dump();
    dump.inputs[0].outputs.push(Output::Log {
        level: LogLevel::Error,
        event: "fabricated".into(),
    });
    assert!(matches!(
        dump.replay(),
        Err(ReplayError::Divergence { index: 0 })
    ));
}
#[test]
fn unknown_dump_version_and_malformed_json_have_clear_errors() {
    let c = Core::new(CoreConfig::default());
    let mut value = serde_json::to_value(Recorder::new(&c).dump()).unwrap();
    value["format_version"] = serde_json::json!(999);
    assert!(
        ReplayDump::from_json(&value.to_string())
            .unwrap_err()
            .to_string()
            .contains("dump version 999")
    );
    assert!(ReplayDump::from_json("{").is_err());
}
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(2000))]
    #[test]
    fn finite_numbers_survive_json_replay_exactly(value in proptest::prelude::any::<f64>().prop_filter("finite", |v| v.is_finite())) {
        let mut c=Core::new(CoreConfig::default());let mut recorder=Recorder::new(&c);
        recorder.handle(&mut c,Timestamp(0),Input::Fact {name:"value".into(),value:Some(FactValue::Number(value))});
        let decoded=ReplayDump::from_json(&recorder.dump().to_json().unwrap()).unwrap();
        proptest::prop_assert_eq!(decoded.replay().unwrap().core.snapshot(),c.snapshot());
    }
}
