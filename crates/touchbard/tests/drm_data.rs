use touchbar_protocol::OutputTransform;
use touchbard::drm::{
    dimensions::{Measurement, SizeSource, resolve},
    events::decode_events,
};
fn flip(sec: u32, micros: u32, crtc: u32) -> Vec<u8> {
    let mut b = vec![0; 32];
    b[..4].copy_from_slice(&2u32.to_ne_bytes());
    b[4..8].copy_from_slice(&32u32.to_ne_bytes());
    b[16..20].copy_from_slice(&sec.to_ne_bytes());
    b[20..24].copy_from_slice(&micros.to_ne_bytes());
    b[28..32].copy_from_slice(&crtc.to_ne_bytes());
    b
}
#[test]
fn drm_events_are_checked_before_reading_fields_and_zero_crtc_does_not_panic() {
    let bytes = flip(123, 456, 0);
    let events = decode_events(&bytes).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].crtc, 0);
    assert_eq!(events[0].timestamp_us, 123000456);
    for bytes in [
        vec![0; 7],
        vec![0; 8],
        flip(1, 1000000, 1),
        bytes[..31].to_vec(),
    ] {
        assert!(decode_events(&bytes).is_err());
    }
    let mut batch = flip(1, 2, 3);
    batch.extend(flip(2, 3, 4));
    assert_eq!(decode_events(&batch).unwrap().len(), 2);
}
#[test]
fn physical_size_prefers_connector_then_measured_dmi_then_explicit_fallback() {
    let measured = [Measurement {
        model: "TestBook",
        width_mm100: 33000,
        height_mm100: 1000,
    }];
    let size = resolve(
        (2008, 60),
        OutputTransform::QuarterTurn,
        Some((10, 334)),
        Some("TestBook"),
        &measured,
    )
    .unwrap();
    assert_eq!(
        (size.width_mm100, size.height_mm100, size.source),
        (33400, 1000, SizeSource::Connector)
    );
    let size = resolve(
        (2008, 60),
        OutputTransform::QuarterTurn,
        Some((0, 0)),
        Some("TestBook"),
        &measured,
    )
    .unwrap();
    assert_eq!(
        (size.width_mm100, size.height_mm100, size.source),
        (33000, 1000, SizeSource::Dmi)
    );
    let size = resolve(
        (2008, 60),
        OutputTransform::Identity,
        None,
        Some("Unknown"),
        &measured,
    )
    .unwrap();
    assert_eq!(
        (size.width_mm100, size.height_mm100, size.source),
        (33467, 1000, SizeSource::Fallback)
    );
    assert!(size.warning.unwrap().contains("Unknown"));
}
proptest::proptest! {#[test] fn arbitrary_drm_events_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(),0..1024)) {let _=decode_events(&bytes);}}
