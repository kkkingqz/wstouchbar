use proptest::prelude::*;
use touchbar_effects::dbus::codec::*;
use touchbar_model::DbusValue;
fn roundtrip(signature: &str, json: serde_json::Value) {
    let input = DbusValue {
        signature: signature.into(),
        json: json.to_string(),
    };
    let body = encode(&input).unwrap();
    let message = zbus::Message::method_call("/test", "Roundtrip")
        .unwrap()
        .build(&body)
        .unwrap();
    let output = decode_message(&message).unwrap();
    assert_eq!(output.signature, input.signature);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output.json).unwrap(),
        json
    );
}
#[test]
fn nested_struct_arrays_dictionary_variants_and_integer_widths_cross_real_dbus_bytes() {
    roundtrip(
        "(is)a{sv}ay",
        serde_json::json!([[42,"hello"],{"text":{"sig":"s","value":"value"},"nested":{"sig":"v","value":{"sig":"u","value":42}}},[0,127,255]]),
    );
    roundtrip(
        "ybnqiuxtdso g".replace(' ', "").as_str(),
        serde_json::json!([
            255,
            true,
            -32768,
            65535,
            -2147483648,
            4294967295u64,
            -9223372036854775808i64,
            18446744073709551615u64,
            1.25,
            "text",
            "/valid/path",
            "a{sv}"
        ]),
    );
    roundtrip("", serde_json::json!([]));
}
#[test]
fn invalid_shapes_numbers_paths_variants_and_fds_are_rejected() {
    for (signature, json) in [
        ("y", "[256]"),
        ("i", "[2147483648]"),
        ("o", "[\"not/path\"]"),
        ("s", "[\"bad\\u0000string\"]"),
        ("v", "[{\"sig\":\"ii\",\"value\":[1,2]}]"),
        ("h", "[1]"),
        ("a{uv}", "[{\"01\":{\"sig\":\"s\",\"value\":\"value\"}}]"),
        ("s", "[]"),
        ("s", "\"scalar\""),
    ] {
        assert!(
            encode(&DbusValue {
                signature: signature.into(),
                json: json.into()
            })
            .is_err(),
            "{signature}: {json}"
        );
    }
}
proptest! {#[test]fn typed_json_roundtrips_through_the_native_wire(a in any::<i32>(),b in prop::collection::vec(any::<u16>(),0..32),s in "[a-zA-Z0-9 _-]{0,64}"){roundtrip("iaq(sv)",serde_json::json!([a,b,[s,{"sig":"b","value":true}]]));}}
#[test]
fn complete_message_signature_can_use_all_255_bytes() {
    let sig = "s".repeat(255);
    roundtrip(
        &sig,
        serde_json::Value::Array(vec![serde_json::Value::String(String::new()); 255]),
    );
}
