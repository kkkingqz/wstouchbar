use touchbar_audio::*;
use touchbar_model::*;
#[test]
fn unavailable_remote_starts_with_snapshot_and_stops_promptly() {
    let start = std::time::Instant::now();
    let backend = NativeAudio::connect(NativeOptions {
        remote: Some("/nonexistent/wstouchbar-test-pipewire".into()),
    })
    .unwrap();
    let watch = backend.watch(AudioTarget::DefaultSink).unwrap();
    let state = watch.events.recv_blocking().unwrap().unwrap();
    assert!(!state.available);
    let result = futures_lite::future::block_on(backend.command(AudioCommand::SetMute {
        target: AudioTarget::DefaultSink,
        value: MuteValue::Toggle,
    }));
    assert_eq!(result, Err(EffectError::Unavailable));
    drop(watch);
    drop(backend);
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
}
