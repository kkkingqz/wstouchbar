use touchbar_core::{Input, KeySource};
use touchbar_model::*;
use touchbar_session::surface::SurfaceEvent;
use touchbar_session::{cli::Cli, simulator::*};
fn info(id: u64, kind: SurfaceKind) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(id),
        name: if kind == SurfaceKind::Touchbar {
            "touchbar"
        } else {
            "window:test"
        }
        .parse()
        .unwrap(),
        kind,
        width_px: 400,
        height_px: 60,
        px_per_mm: 6.2,
        caps: SurfaceCaps {
            watch_keys: kind == SurfaceKind::Touchbar,
            inject_keys: kind == SurfaceKind::Touchbar,
            damage: true,
            ..SurfaceCaps::default()
        },
    }
}
fn inputs(router: &mut InputRouter) -> Vec<Input> {
    router
        .drain()
        .into_iter()
        .filter_map(|e| match e {
            SimulatorEvent::Surface(SurfaceEvent::Input(input)) => Some(input),
            _ => None,
        })
        .collect()
}
#[test]
fn named_surfaces_have_stable_ids_and_realistic_capabilities() {
    let cli = Cli::parse(
        [
            "--simulator",
            "--surface",
            "touchbar=2008x60@6.2ppmm",
            "--surface",
            "window:test=1200x160@4ppmm",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .unwrap();
    assert_eq!(cli.surfaces.len(), 2);
    assert_eq!(cli.surfaces[0].kind, SurfaceKind::Touchbar);
    assert!(cli.surfaces[0].caps.watch_keys && cli.surfaces[0].caps.inject_keys);
    assert!(!cli.surfaces[0].caps.modes);
    assert!(!cli.surfaces[1].caps.watch_keys && !cli.surfaces[1].caps.inject_keys);
    assert_ne!(cli.surfaces[0].id, cli.surfaces[1].id);
    assert!(
        Cli::parse(
            [
                "--simulator",
                "--surface",
                "touchbar=400x60@6ppmm",
                "--surface",
                "touchbar=800x60@6ppmm"
            ]
            .into_iter()
            .map(str::to_owned)
        )
        .is_err()
    );
}
#[test]
fn mouse_coordinates_follow_scale_and_shift_contact_stays_fixed_until_up() {
    let mut router = InputRouter::default();
    router.add(info(1, SurfaceKind::Touchbar), 2);
    router.keyboard_enter(SurfaceId(1), &[]);
    router.pointer_enter(SurfaceId(1), 10.0, 5.0);
    router.key(42, true);
    router.button(true);
    router.key(42, false);
    router.pointer_motion(20.0, 10.0);
    router.button(false);
    let events = inputs(&mut router);
    let points = events
        .iter()
        .filter_map(|e| {
            if let Input::Touch {
                contact,
                phase,
                x,
                y,
                ..
            } = e
            {
                Some((contact.0.get(), *phase, *x, *y))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        points,
        [
            (2, TouchPhase::Down, 20.0, 10.0),
            (2, TouchPhase::Move, 40.0, 20.0),
            (2, TouchPhase::Up, 40.0, 20.0)
        ]
    );
    router.pointer_enter(SurfaceId(1), 5.0, 5.0);
    router.button(true);
    router.pointer_motion(-10.0, 40.0);
    router.button(false);
    assert!(
        inputs(&mut router)
            .iter()
            .any(|i| matches!(i,Input::Touch{phase:TouchPhase::Up,x,..}if *x<0.0))
    );
}
#[test]
fn watched_keys_focus_loss_and_theme_hotkey_are_deterministic() {
    let mut router = InputRouter::default();
    router.add(info(1, SurfaceKind::Touchbar), 1);
    router.watch(SurfaceId(1), &[WatchKey::Fn, WatchKey::F1, WatchKey::Esc]);
    router.keyboard_enter(SurfaceId(1), &[]);
    router.key(100, true);
    router.key(59, true);
    router.key(59, true);
    router.keyboard_leave();
    let events = inputs(&mut router);
    assert_eq!(
        events
            .iter()
            .filter(|i| matches!(
                i,
                Input::Key {
                    code: WatchKey::F1,
                    phase: KeyPhase::Pressed,
                    ..
                }
            ))
            .count(),
        1
    );
    assert!(events.iter().any(|i| matches!(
        i,
        Input::Key {
            source: KeySource::Surface(SurfaceId(1)),
            code: WatchKey::Fn,
            phase: KeyPhase::Released
        }
    )));
    router.keyboard_enter(SurfaceId(1), &[]);
    router.key(29, true);
    router.key(20, true);
    router.key(20, true);
    assert_eq!(
        router
            .drain()
            .iter()
            .filter(|e| matches!(e, SimulatorEvent::ToggleTheme))
            .count(),
        1
    );
}
#[test]
fn native_touches_do_not_alias_mouse_and_window_does_not_claim_watch_keys() {
    let mut router = InputRouter::default();
    router.add(info(1, SurfaceKind::Touchbar), 1);
    router.add(info(2, SurfaceKind::Window), 1);
    router.keyboard_enter(SurfaceId(2), &[]);
    router.watch(SurfaceId(2), &[WatchKey::Fn]);
    router.key(100, true);
    assert!(inputs(&mut router).is_empty());
    router.touch_down(SurfaceId(1), 0, 10.0, 10.0);
    router.touch_down(SurfaceId(2), 1, 20.0, 10.0);
    router.touch_motion(0, 30.0, 20.0);
    router.touch_up(1);
    router.touch_cancel();
    let events = inputs(&mut router);
    assert!(
        events
            .iter()
            .all(|e| matches!(e,Input::Touch{contact,..}if contact.0.get()>=3))
    );
    assert!(events.iter().any(|e| matches!(
        e,
        Input::Touch {
            surface: SurfaceId(1),
            phase: TouchPhase::Cancel,
            ..
        }
    )));
}
#[test]
fn shm_argb_keeps_premultiplication_and_checked_geometry() {
    let bytes = argb8888(&[128, 32, 8, 128, 255, 0, 0, 255]).unwrap();
    assert_eq!(
        u32::from_ne_bytes(bytes[..4].try_into().unwrap()),
        0x80802008
    );
    assert_eq!(
        u32::from_ne_bytes(bytes[4..].try_into().unwrap()),
        0xffff0000
    );
    assert!(argb8888(&[1, 2, 3]).is_err());
    assert_eq!(physical_size((201, 31), 2).unwrap(), (402, 62));
    assert!(physical_size((8192, 8192), 8).is_err());
}
#[test]
fn missing_wayland_socket_is_reported_without_touching_ambient_display() {
    let infos = vec![info(1, SurfaceKind::Touchbar)];
    assert!(
        futures_lite::future::block_on(Simulator::connect_at(
            std::path::Path::new("/nonexistent/wstouchbar-wayland-test"),
            infos
        ))
        .is_err()
    );
}
#[test]
fn simulator_never_delegates_keys_or_shortcuts_but_keeps_other_desktop_commands() {
    use touchbar_desktop::{DesktopBackend, DesktopCommand, FakeBackend};
    let fake = FakeBackend::default();
    let mut desktop = SimulatorDesktop::new(Box::new(fake.clone()));
    let (tx, _rx) = async_channel::bounded(32);
    desktop.start(tx).unwrap();
    futures_lite::future::block_on(async {
        desktop
            .command(DesktopCommand::Key {
                key: SystemKey::F1,
                phase: KeyPhase::Pressed,
            })
            .await
            .unwrap();
        desktop
            .command(DesktopCommand::Shortcut {
                keys: vec!["Ctrl".into(), "Q".into()],
                app: None,
            })
            .await
            .unwrap();
        desktop
            .command(DesktopCommand::BrightnessSet { level: 0.4 })
            .await
            .unwrap();
    });
    assert_eq!(
        fake.take_log(),
        vec![DesktopCommand::BrightnessSet { level: 0.4 }]
    );
}
#[test]
fn losing_pointer_device_cancels_contact_and_clears_pointer_focus() {
    let mut router = InputRouter::default();
    router.add(info(1, SurfaceKind::Touchbar), 1);
    router.pointer_enter(SurfaceId(1), 5.0, 5.0);
    router.button(true);
    inputs(&mut router);
    router.pointer_unavailable();
    assert!(matches!(
        inputs(&mut router).as_slice(),
        [Input::Touch {
            phase: TouchPhase::Cancel,
            ..
        }]
    ));
    router.button(true);
    assert!(inputs(&mut router).is_empty());
}
