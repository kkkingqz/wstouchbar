use touchbar_core::*;
use touchbar_model::*;
fn info(id: u64, inject: bool) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(id),
        name: "touchbar".parse().unwrap(),
        kind: SurfaceKind::Touchbar,
        width_px: 2008,
        height_px: 60,
        px_per_mm: 6.0,
        caps: SurfaceCaps {
            inject_keys: inject,
            ..SurfaceCaps::default()
        },
    }
}
fn owner(number: u64) -> KeyOwner {
    KeyOwner::Logic(InstanceId {
        number,
        generation: Generation(1),
    })
}
#[test]
fn unsupported_key_route_does_not_create_lease() {
    let mut keys = KeyTracker::default();
    assert_eq!(
        keys.press(
            owner(1),
            &info(1, false),
            Mode::Display,
            DesktopCapabilities::default(),
            SystemKey::F1
        ),
        Err(EffectError::Unsupported)
    );
    assert!(keys.release_surface(SurfaceId(1)).is_empty());
}
#[test]
fn key_without_surface_injection_routes_to_desktop() {
    let mut keys = KeyTracker::default();
    assert_eq!(
        keys.press(
            owner(1),
            &info(1, false),
            Mode::Display,
            DesktopCapabilities { key: true },
            SystemKey::F1
        )
        .unwrap(),
        vec![Output::DesktopKey {
            surface: SurfaceId(1),
            key: SystemKey::F1,
            phase: KeyPhase::Pressed
        }]
    );
    assert_eq!(
        keys.release_surface(SurfaceId(1)),
        vec![Output::DesktopKey {
            surface: SurfaceId(1),
            key: SystemKey::F1,
            phase: KeyPhase::Released
        }]
    );
}
#[test]
fn native_refuses_new_presses_but_allows_lease_cleanup() {
    let mut keys = KeyTracker::default();
    keys.press(
        owner(1),
        &info(1, true),
        Mode::Display,
        DesktopCapabilities::default(),
        SystemKey::F1,
    )
    .unwrap();
    assert_eq!(
        keys.press(
            owner(2),
            &info(1, true),
            Mode::Native,
            DesktopCapabilities::default(),
            SystemKey::F2
        ),
        Err(EffectError::Unavailable)
    );
    assert_eq!(
        keys.release_surface(SurfaceId(1)),
        vec![Output::SendKey {
            surface: SurfaceId(1),
            key: SystemKey::F1,
            phase: KeyPhase::Released
        }]
    );
}
#[test]
fn shared_key_is_released_when_last_owner_ends() {
    let mut keys = KeyTracker::default();
    keys.press(
        owner(1),
        &info(1, true),
        Mode::Display,
        DesktopCapabilities::default(),
        SystemKey::F1,
    )
    .unwrap();
    assert!(
        keys.press(
            owner(2),
            &info(1, true),
            Mode::Display,
            DesktopCapabilities::default(),
            SystemKey::F1
        )
        .unwrap()
        .is_empty()
    );
    assert!(keys.release_owner(owner(1)).is_empty());
    assert_eq!(
        keys.release_owner(owner(2)),
        vec![Output::SendKey {
            surface: SurfaceId(1),
            key: SystemKey::F1,
            phase: KeyPhase::Released
        }]
    );
    assert!(keys.release_owner(owner(2)).is_empty());
}
#[test]
fn surface_cleanup_does_not_release_another_surface_key() {
    let mut keys = KeyTracker::default();
    for n in [1, 2] {
        keys.press(
            owner(n),
            &info(n, true),
            Mode::Display,
            DesktopCapabilities::default(),
            SystemKey::F1,
        )
        .unwrap();
    }
    assert_eq!(
        keys.release_surface(SurfaceId(1)),
        vec![Output::SendKey {
            surface: SurfaceId(1),
            key: SystemKey::F1,
            phase: KeyPhase::Released
        }]
    );
    assert_eq!(
        keys.release_surface(SurfaceId(2)),
        vec![Output::SendKey {
            surface: SurfaceId(2),
            key: SystemKey::F1,
            phase: KeyPhase::Released
        }]
    );
}
#[test]
fn owner_generation_distinguishes_recreated_logic() {
    let mut keys = KeyTracker::default();
    keys.press(
        owner(1),
        &info(1, true),
        Mode::Display,
        DesktopCapabilities::default(),
        SystemKey::F1,
    )
    .unwrap();
    assert!(
        keys.release_owner(KeyOwner::Logic(InstanceId {
            number: 1,
            generation: Generation(2)
        }))
        .is_empty()
    );
    assert_eq!(keys.release_owner(owner(1)).len(), 1);
}
