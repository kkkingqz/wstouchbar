use touchbar_model::*;

#[test]
fn nonfinite_values_preserve_width_sign_and_nan_payload() {
    for bits in [0x7f800000, 0xff800000, 0x7fc00001, 0xff800001, 0x7fffffff] {
        let point = Point {
            x: f32::from_bits(bits),
            y: -0.0,
        };
        let json = serde_json::to_string(&point).unwrap();
        assert!(json.contains(&format!("f32:{bits:08x}")));
        let decoded: Point = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.x.to_bits(), bits);
        assert_eq!(decoded.y.to_bits(), (-0.0_f32).to_bits());
    }
    for bits in [
        0x7ff0000000000000,
        0xfff0000000000000,
        0x7ff8000000000001,
        0xfff0000000000001,
        0x7fffffffffffffff,
    ] {
        let scalar = Scalar::Number(f64::from_bits(bits));
        let json = serde_json::to_string(&scalar).unwrap();
        assert_eq!(json, format!(r#"{{"Number":"f64:{bits:016x}"}}"#));
        let Scalar::Number(decoded) = serde_json::from_str(&json).unwrap() else {
            panic!("number");
        };
        assert_eq!(decoded.to_bits(), bits);
    }
}

#[test]
fn finite_numbers_stay_numbers_and_invalid_tags_are_rejected() {
    for value in [0.0_f64, -0.0, 0.5, f64::MAX, f64::MIN_POSITIVE] {
        let scalar = Scalar::Number(value);
        let json = serde_json::to_value(&scalar).unwrap();
        assert!(json["Number"].is_number());
        let Scalar::Number(decoded) = serde_json::from_value(json).unwrap() else {
            panic!("number");
        };
        assert_eq!(decoded.to_bits(), value.to_bits());
    }
    for tag in [
        "NaN",
        "Infinity",
        "f32:7fc00000",
        "f64:7ff",
        "f64:+ff8000000000000",
        "f64:7ff800000000000g",
        "f64:3ff0000000000000",
    ] {
        let json = serde_json::json!({"Number":tag});
        assert!(serde_json::from_value::<Scalar>(json).is_err(), "{tag}");
    }
    assert!(serde_json::from_str::<Scalar>(r#"{"Number":null}"#).is_err());
    assert!(serde_json::from_str::<Point>(r#"{"x":"f32:3f800000","y":0}"#).is_err());
}

#[test]
fn encoding_nonfinite_values_does_not_bypass_domain_validation() {
    let info = SurfaceInfo {
        px_per_mm: f32::INFINITY,
        ..surface()
    };
    let decoded: SurfaceInfo =
        serde_json::from_str(&serde_json::to_string(&info).unwrap()).unwrap();
    assert!(decoded.validate().is_err());
    assert!(Length::Unit(f32::NAN).validate().is_err());
    assert!(serde_json::to_string(&Length::Unit(f32::NAN)).is_err());
}

fn surface() -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(1),
        name: "headless:test".parse().unwrap(),
        kind: SurfaceKind::Headless,
        width_px: 500,
        height_px: 50,
        px_per_mm: 5.0,
        caps: SurfaceCaps::default(),
    }
}
