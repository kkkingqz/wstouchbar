use std::collections::BTreeMap;
use touchbar_model::*;

fn surface(width: u32, height: u32) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(1),
        name: "touchbar".parse().unwrap(),
        kind: SurfaceKind::Touchbar,
        width_px: width,
        height_px: height,
        px_per_mm: 6.0,
        caps: SurfaceCaps::default(),
    }
}

#[test]
fn lengths_parse_and_convert_relative_to_surface() {
    let s = surface(2008, 60);
    let unit = UnitSize::resolve(&s, None, None);
    for (json, px) in [
        ("200", 120.0),
        ("\"18u\"", 10.8),
        ("\"6%w\"", 120.48),
        ("\"0.5%h\"", 0.3),
    ] {
        let length: Length = serde_json::from_str(json).unwrap();
        assert!((length.to_px(&s, unit) - px).abs() < 0.001);
        let encoded = serde_json::to_string(&length).unwrap();
        assert_eq!(serde_json::from_str::<Length>(&encoded).unwrap(), length);
    }
}

#[test]
fn invalid_lengths_cannot_enter_through_deserialization() {
    for json in [
        "-1",
        "10001",
        "\"6px\"",
        "\"101%w\"",
        "\"-1%h\"",
        "\"NaNu\"",
        "\"infu\"",
        "true",
        "null",
    ] {
        assert!(serde_json::from_str::<Length>(json).is_err(), "{json}");
    }
    for json in ["200", "\"18u\"", "\"0%w\"", "\"101%h\""] {
        assert!(serde_json::from_str::<UnitSize>(json).is_err(), "{json}");
    }
}

#[test]
fn units_resolve_set_then_profile_then_short_side() {
    let s = surface(2000, 60);
    let set: UnitSize = serde_json::from_str("\"0.05%w\"").unwrap();
    let profile: UnitSize = serde_json::from_str("\"2%h\"").unwrap();
    let l: Length = serde_json::from_str("200").unwrap();
    for (a, b, want) in [
        (Some(set), Some(profile), 200.0),
        (None, Some(profile), 240.0),
        (None, None, 120.0),
    ] {
        assert!((l.to_px(&s, UnitSize::resolve(&s, a, b)) - want).abs() < 0.001);
    }
    let portrait = surface(60, 2000);
    assert!((l.to_px(&portrait, UnitSize::resolve(&portrait, None, None)) - 120.0).abs() < 0.001);
    assert_eq!(Mm(1.3).to_px(10.0), 13.0);
}

#[test]
fn conditions_evaluate_absent_and_composed_facts() {
    let facts = BTreeMap::from([
        (FactName::from("app.id"), FactValue::Text("firefox".into())),
        (FactName::from("media.active"), FactValue::Boolean(true)),
    ]);
    let eq = Condition::Eq {
        name: "media.active".into(),
        value: FactValue::Boolean(true),
    };
    let absent = Condition::Exists("missing".into());
    let member = Condition::In {
        name: "app.id".into(),
        values: vec![
            FactValue::Text("firefox".into()),
            FactValue::Text("vlc".into()),
        ],
    };
    assert!(eq.evaluate(&facts));
    assert!(!absent.evaluate(&facts));
    assert!(
        Condition::All(vec![
            eq.clone(),
            member.clone(),
            Condition::Not(Box::new(absent))
        ])
        .evaluate(&facts)
    );
    assert!(Condition::Any(vec![Condition::Exists("missing".into()), member]).evaluate(&facts));
    assert!(
        !Condition::Eq {
            name: "missing".into(),
            value: FactValue::Boolean(false)
        }
        .evaluate(&facts)
    );
    assert!(Condition::All(vec![]).evaluate(&facts));
    assert!(!Condition::Any(vec![]).evaluate(&facts));
}

#[test]
fn condition_depth_is_limited_to_eight() {
    let mut c = Condition::Exists("app.id".into());
    for _ in 0..7 {
        c = Condition::Not(Box::new(c));
    }
    assert!(c.validate().is_ok());
    assert!(Condition::Not(Box::new(c)).validate().is_err());
}

#[test]
fn surface_facts_follow_density_and_size() {
    let facts = surface(2008, 60).facts();
    assert_eq!(
        facts[&FactName::from("surface.kind")],
        FactValue::Text("touchbar".into())
    );
    assert_eq!(
        facts[&FactName::from("surface.height-mm")],
        FactValue::Number(10.0)
    );
    assert!(
        (match facts[&FactName::from("surface.aspect")] {
            FactValue::Number(n) => n,
            _ => panic!(),
        } - 2008.0 / 60.0)
            .abs()
            < 0.001
    );
    let mut s = surface(2008, 60);
    s.px_per_mm = 0.0;
    assert!(s.validate().is_err());
    s.px_per_mm = f32::NAN;
    assert!(s.validate().is_err());
}

#[test]
fn identifiers_reject_invalid_paths_and_zero_contact() {
    for s in ["", "../controls", "Controls", "foo/bar"] {
        assert!(s.parse::<PluginId>().is_err());
    }
    for s in [
        "volume",
        "controls/",
        "/volume",
        "controls/a/b",
        "../volume",
    ] {
        assert!(s.parse::<SetRef>().is_err());
    }
    let r: SetRef = "controls/volume".parse().unwrap();
    assert_eq!(serde_json::to_string(&r).unwrap(), "\"controls/volume\"");
    assert_eq!(
        serde_json::from_str::<SetRef>("\"controls/volume\"").unwrap(),
        r
    );
    assert!(serde_json::from_str::<ContactId>("0").is_err());
    assert!(serde_json::from_str::<PluginId>("\"../controls\"").is_err());
}
#[test]
fn bounds_are_checked_before_narrowing_to_f32() {
    for json in ["10000.000001", "\"100.000001%w\"", "-1e-50"] {
        assert!(serde_json::from_str::<Length>(json).is_err(), "{json}");
    }
}
#[test]
fn conditions_reject_nonfinite_numeric_comparisons() {
    assert!(
        Condition::Eq {
            name: "volume".into(),
            value: FactValue::Number(f64::NAN)
        }
        .validate()
        .is_err()
    );
    assert!(
        Condition::In {
            name: "volume".into(),
            values: vec![FactValue::Number(f64::INFINITY)]
        }
        .validate()
        .is_err()
    );
}
