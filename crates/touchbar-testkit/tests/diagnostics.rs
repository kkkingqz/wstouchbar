use touchbar_core::*;
use touchbar_model::*;
use touchbar_testkit::*;
fn setup() -> Harness {
    let mut h = Harness::new(CoreConfig::default());
    h.send(Input::Catalog(fixtures::controls_with_panel()));
    h.send(Input::SurfaceAdded(headless(1)));
    h
}
#[test]
fn completed_capture_records_original_target_once() {
    let mut h = setup();
    let point = h.hit(SurfaceId(1), "microphone");
    h.touch(SurfaceId(1), 1, TouchPhase::Down, point);
    let capture = h.snapshot().captures[0].clone();
    let start = h.trace.len();
    h.touch(
        SurfaceId(1),
        1,
        TouchPhase::Up,
        Point {
            x: point.x + 300.0,
            y: point.y,
        },
    );
    let out: Vec<_> = h.trace[start..]
        .iter()
        .flat_map(|e| e.outputs.clone())
        .collect();
    let completions: Vec<_> = out
        .iter()
        .filter_map(|o| {
            if let Output::CaptureFinished { capture, .. } = o {
                Some(capture)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(completions, vec![&capture]);
    assert!(out.contains(&Output::CaptureFinished {
        capture,
        completion: CaptureCompletion::Up
    }));
    let out = h.send(Input::Touch {
        surface: SurfaceId(1),
        contact: ContactId::new(1).unwrap(),
        phase: TouchPhase::Up,
        x: point.x,
        y: point.y,
    });
    assert!(
        out.iter()
            .all(|o| !matches!(o, Output::CaptureFinished { .. }))
    );
}
#[test]
fn removal_records_cancel_and_exposes_shared_desktop_key_owners() {
    let mut h = setup();
    let mut first = headless(1);
    first.caps.inject_keys = false;
    h.send(Input::SurfaceChanged(first));
    let mut second = headless(2);
    second.caps.inject_keys = false;
    h.send(Input::SurfaceAdded(second));
    h.send(Input::DesktopCapabilities(DesktopCapabilities {
        key: true,
    }));
    for surface in [SurfaceId(1), SurfaceId(2)] {
        h.send(Input::Key {
            source: KeySource::Surface(surface),
            code: WatchKey::Fn,
            phase: KeyPhase::Pressed,
        });
        h.touch(surface, 1, TouchPhase::Down, h.hit(surface, "f1"));
    }
    assert_eq!(h.snapshot().key_leases.len(), 2);
    assert!(
        h.snapshot()
            .key_leases
            .iter()
            .all(|l| l.destination == KeyDestination::Desktop)
    );
    let capture = h
        .snapshot()
        .captures
        .iter()
        .find(|c| c.surface == SurfaceId(1))
        .unwrap()
        .clone();
    let out = h.send(Input::SurfaceRemoved(SurfaceId(1)));
    assert!(out.contains(&Output::CaptureFinished {
        capture,
        completion: CaptureCompletion::Cancel(CancelReason::SurfaceRemoved)
    }));
    assert!(!out.iter().any(|o| matches!(
        o,
        Output::DesktopKey {
            phase: KeyPhase::Released,
            ..
        }
    )));
    assert_eq!(h.snapshot().key_leases.len(), 1);
    assert_eq!(h.snapshot().key_leases[0].surface, SurfaceId(2));
    let out = h.send(Input::SurfaceRemoved(SurfaceId(2)));
    assert!(out.iter().any(|o| matches!(
        o,
        Output::DesktopKey {
            key: SystemKey::F1,
            phase: KeyPhase::Released,
            ..
        }
    )));
    assert!(h.snapshot().key_leases.is_empty());
}

#[test]
fn native_transition_preserves_mode_reason_before_fn_layer_teardown() {
    let mut h = setup();
    let mut info = headless(1);
    info.caps.modes = true;
    h.send(Input::SurfaceChanged(info));
    h.send(Input::Key {
        source: KeySource::Surface(SurfaceId(1)),
        code: WatchKey::Fn,
        phase: KeyPhase::Pressed,
    });
    h.touch(SurfaceId(1), 1, TouchPhase::Down, h.hit(SurfaceId(1), "f1"));
    let capture = h.snapshot().captures[0].clone();
    let out = h.send(Input::Mode {
        surface: SurfaceId(1),
        mode: Mode::Native,
    });
    assert!(out.contains(&Output::CaptureFinished {
        capture,
        completion: CaptureCompletion::Cancel(CancelReason::Mode)
    }));
    assert!(h.snapshot().key_leases.is_empty());
}
