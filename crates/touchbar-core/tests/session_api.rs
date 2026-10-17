use touchbar_core::*;
use touchbar_model::*;
#[test]
fn recorder_is_available_without_testkit_and_pending_read_does_not_mutate_core() {
    let mut core = Core::new(CoreConfig::default());
    let mut recorder = Recorder::new(&core);
    recorder.handle(&mut core, Timestamp(0), Input::Locale("uk_UA.UTF-8".into()));
    let before = serde_json::to_string(&core.checkpoint()).unwrap();
    assert!(core.pending_persist().is_empty());
    assert_eq!(serde_json::to_string(&core.checkpoint()).unwrap(), before);
    assert_eq!(recorder.dump().format_version, 4);
    assert_eq!(
        recorder.dump().replay().unwrap().core.snapshot(),
        core.snapshot()
    );
}
