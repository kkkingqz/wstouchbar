use touchbar_core::*;
use touchbar_model::*;
use touchbar_testkit::*;
#[test]
fn rejected_nonfinite_input_remains_replayable() {
    let mut core = Core::new(CoreConfig::default());
    let mut recorder = Recorder::new(&core);
    recorder.handle(
        &mut core,
        Timestamp(0),
        Input::Fact {
            name: "broken".into(),
            value: Some(FactValue::Number(f64::NAN)),
        },
    );
    let json = recorder.dump().to_json().unwrap();
    let replay = ReplayDump::from_json(&json)
        .expect("all recorded inputs must decode, including rejected values")
        .replay()
        .unwrap();
    assert_eq!(replay.core.snapshot(), core.snapshot());
}

#[test]
fn rejected_float32_inputs_and_optional_values_keep_their_bits() {
    let mut core = Core::new(CoreConfig::default());
    let mut recorder = Recorder::new(&core);
    for (n, bits) in [0x7f800000, 0xff800000, 0x7fc12345, 0xff800001]
        .into_iter()
        .enumerate()
    {
        recorder.handle(
            &mut core,
            Timestamp(n as u64),
            Input::Touch {
                surface: SurfaceId(1),
                contact: ContactId::new(1).unwrap(),
                phase: TouchPhase::Down,
                x: f32::from_bits(bits),
                y: 0.0,
            },
        );
    }
    let decoded = ReplayDump::from_json(&recorder.dump().to_json().unwrap()).unwrap();
    for (entry, bits) in decoded
        .inputs
        .iter()
        .zip([0x7f800000, 0xff800000, 0x7fc12345, 0xff800001])
    {
        let Input::Touch { x, .. } = entry.input else {
            panic!("touch");
        };
        assert_eq!(x.to_bits(), bits);
    }
    assert_eq!(decoded.replay().unwrap().core.snapshot(), core.snapshot());
    for value in [
        None,
        Some(f64::from_bits(0xfff8000000000042)),
        Some(f64::INFINITY),
        Some(-0.0),
    ] {
        let interaction = WidgetInteraction {
            value,
            widget: "slider".parse().unwrap(),
            record_key: None,
            pressed: false,
            scrubber: ScrubberState::default(),
        };
        let decoded: WidgetInteraction =
            serde_json::from_str(&serde_json::to_string(&interaction).unwrap()).unwrap();
        assert_eq!(decoded.value.map(f64::to_bits), value.map(f64::to_bits));
    }
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(2000))]
    #[test]
    fn all_float_bits_round_trip(bits32 in proptest::prelude::any::<u32>(), bits64 in proptest::prelude::any::<u64>()) {
        let point = Point { x: f32::from_bits(bits32), y: 0.0 };
        let decoded: Point = serde_json::from_str(&serde_json::to_string(&point).unwrap()).unwrap();
        proptest::prop_assert_eq!(decoded.x.to_bits(), bits32);
        let number = Scalar::Number(f64::from_bits(bits64));
        let Scalar::Number(decoded) = serde_json::from_str(&serde_json::to_string(&number).unwrap()).unwrap() else { panic!("number"); };
        proptest::prop_assert_eq!(decoded.to_bits(), bits64);
    }
}
