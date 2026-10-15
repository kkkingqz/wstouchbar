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
