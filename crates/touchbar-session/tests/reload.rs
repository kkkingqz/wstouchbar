use std::rc::Rc;
use touchbar_loader::*;
use touchbar_session::*;
#[test]
fn invalid_log_filter_rejects_reload_without_changing_accepted_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = Rc::new(touchbar_desktop::NullBackend::default());
    let (effects, _) = touchbar_effects::Engine::fake(desktop.clone());
    let mut session = Session::new(
        SessionOptions::new(dir.path().into(), UserConfig::default()),
        ManualClock::default(),
        Box::<touchbar_logic_runtime::MemoryStore>::default(),
        desktop,
        effects,
    )
    .unwrap();
    let request = LoadRequest {
        paths: vec![dir.path().join("plugins")],
        config_file: dir.path().join("config.toml"),
        bindings_file: dir.path().join("bindings.toml"),
    };
    assert!(
        session
            .load(prepare(request.clone(), session.compiler()))
            .accepted
    );
    session.pump().unwrap();
    std::fs::write(&request.config_file, "log='['\n").unwrap();
    let report = session.load(prepare(request, session.compiler()));
    assert!(!report.accepted, "invalid filter was admitted");
    assert_eq!(session.config().log, "info");
}
#[test]
fn unchanged_reload_does_not_create_replay_noise_or_reset_frames() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = Rc::new(touchbar_desktop::NullBackend::default());
    let (effects, _) = touchbar_effects::Engine::fake(desktop.clone());
    let mut session = Session::new(
        SessionOptions::new(dir.path().into(), UserConfig::default()),
        ManualClock::default(),
        Box::<touchbar_logic_runtime::MemoryStore>::default(),
        desktop,
        effects,
    )
    .unwrap();
    let request = LoadRequest {
        paths: vec![dir.path().join("plugins")],
        config_file: dir.path().join("config.toml"),
        bindings_file: dir.path().join("bindings.toml"),
    };
    assert!(
        session
            .load(prepare(request.clone(), session.compiler()))
            .accepted
    );
    session.pump().unwrap();
    let before = session.dump().inputs.len();
    assert!(session.load(prepare(request, session.compiler())).accepted);
    session.pump().unwrap();
    assert_eq!(session.dump().inputs.len(), before);
}
#[test]
fn empty_startup_catalog_keeps_scene_for_later_theme_repaint() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = Rc::new(touchbar_desktop::NullBackend::default());
    let (effects, _) = touchbar_effects::Engine::fake(desktop.clone());
    let mut session = Session::new(
        SessionOptions::new(dir.path().into(), UserConfig::default()),
        ManualClock::default(),
        Box::<touchbar_logic_runtime::MemoryStore>::default(),
        desktop,
        effects,
    )
    .unwrap();
    let info = touchbar_session::cli::parse_surface(
        "400x60@6.2ppmm",
        touchbar_model::SurfaceKind::Headless,
    )
    .unwrap();
    let (surface, handle) = touchbar_session::surface::HeadlessSurface::new(info);
    session.add_surface(Box::new(surface)).unwrap();
    session.pump().unwrap();
    let before = handle.frame().unwrap().display_list.clone();
    let request = LoadRequest {
        paths: vec![dir.path().join("plugins")],
        config_file: dir.path().join("config.toml"),
        bindings_file: dir.path().join("bindings.toml"),
    };
    assert!(session.load(prepare(request, session.compiler())).accepted);
    session.pump().unwrap();
    session
        .handle_event(Event::Input(touchbar_core::Input::Fact {
            name: "appearance.scheme".into(),
            value: Some(touchbar_model::FactValue::Text("light".into())),
        }))
        .unwrap();
    session.pump().unwrap();
    assert_ne!(handle.frame().unwrap().display_list, before);
}
