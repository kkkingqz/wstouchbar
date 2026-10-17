use touchbar_protocol::{DamageRect, damage::Damage};
use touchbard::drm::{Frame, Presentation};
#[test]
fn first_full_then_damage_and_release_only_after_completion() {
    let mut p = Presentation::new(3).unwrap();
    let r = DamageRect {
        x: 1,
        y: 2,
        width: 3,
        height: 4,
    };
    assert_eq!(
        p.begin(Frame { index: 0, frame: 1 }, Damage::Rects(vec![r]))
            .unwrap(),
        Damage::Full
    );
    assert!(p.begin(Frame { index: 1, frame: 2 }, Damage::Full).is_err());
    let a = p.complete().unwrap();
    assert_eq!(a.presented, Frame { index: 0, frame: 1 });
    assert_eq!(a.released, None);
    assert!(p.begin(Frame { index: 0, frame: 2 }, Damage::Full).is_err());
    assert_eq!(
        p.begin(Frame { index: 1, frame: 2 }, Damage::Rects(vec![r]))
            .unwrap(),
        Damage::Rects(vec![r])
    );
    let b = p.complete().unwrap();
    assert_eq!(b.released, Some(Frame { index: 0, frame: 1 }));
    assert!(p.complete().is_err());
    assert!(p.begin(Frame { index: 2, frame: 2 }, Damage::Full).is_err());
    p.begin(Frame { index: 0, frame: 3 }, Damage::Full).unwrap();
}
#[test]
fn failed_submission_keeps_displayed_buffer_and_forces_full_retry() {
    let mut p = Presentation::new(2).unwrap();
    p.begin(Frame { index: 0, frame: 1 }, Damage::Full).unwrap();
    p.complete().unwrap();
    p.begin(Frame { index: 1, frame: 2 }, Damage::Full).unwrap();
    p.failed();
    assert!(p.begin(Frame { index: 0, frame: 3 }, Damage::Full).is_err());
    assert_eq!(
        p.begin(Frame { index: 1, frame: 3 }, Damage::Rects(vec![]))
            .unwrap(),
        Damage::Full
    );
}
