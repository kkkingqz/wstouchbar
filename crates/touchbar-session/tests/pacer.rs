use touchbar_model::Timestamp;
use touchbar_session::surface::FramePacer;
#[test]
fn one_frame_in_flight_and_animation_never_exceeds_sixty_hz() {
    let mut frames = FramePacer::default();
    assert!(frames.begin(Timestamp(0), false));
    assert!(!frames.begin(Timestamp(1), false));
    assert!(!frames.begin(Timestamp(100), true));
    frames.finish(true);
    assert_eq!(frames.deadline(), Some(Timestamp(17)));
    assert!(!frames.begin(Timestamp(16), true));
    assert!(frames.begin(Timestamp(17), true));
    frames.finish(true);
    let mut count = 2;
    for ms in 18..1000 {
        if frames.begin(Timestamp(ms), true) {
            count += 1;
            frames.finish(true);
        }
    }
    assert!(count <= 60);
    frames.cancel();
    assert_eq!(frames.deadline(), None);
}
