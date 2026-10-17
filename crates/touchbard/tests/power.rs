use touchbar_model::Timestamp;
use touchbard::{
    config::BacklightLevels,
    machine::power::{Bridge, Power},
};
#[test]
fn dimming_has_finite_deadlines_and_returns_to_active_on_activity() {
    let mut p = Power::new(BacklightLevels::default(), Timestamp(0));
    assert_eq!(p.next_deadline(Timestamp(0)), None);
    p.set_timeouts(Some(1000), Some(2000));
    assert_eq!(p.level(Timestamp(999)), 2);
    assert_eq!(p.level(Timestamp(1000)), 1);
    assert_eq!(p.level(Timestamp(3000)), 0);
    assert_eq!(p.next_deadline(Timestamp(3000)), None);
    p.activity(Timestamp(4000));
    assert_eq!(p.level(Timestamp(4000)), 2);
    assert_eq!(p.next_deadline(Timestamp(4000)), Some(Timestamp(5000)));
    p.lid(true);
    assert_eq!(p.level(Timestamp(4000)), 0);
    assert_eq!(p.next_deadline(Timestamp(4000)), None);
    p.lid(false);
    p.limit(32768);
    assert_eq!(p.level(Timestamp(4000)), 1);
    p.limit(0);
    assert_eq!(p.level(Timestamp(4000)), 0);
}
#[test]
fn bridge_restores_native_driver_on_release_mode_exit_and_shutdown() {
    for exit in [(true, false), (false, true)] {
        let mut b = Bridge::default();
        assert_eq!(b.reconcile(true, true, Some(1)), Some(2));
        assert_eq!(b.reconcile(true, true, Some(2)), None);
        assert_eq!(b.reconcile(exit.0, exit.1, Some(2)), Some(1));
        assert_eq!(b.reconcile(false, false, Some(1)), None);
    }
    for mode in [0, 3] {
        assert_eq!(Bridge::default().reconcile(true, true, Some(mode)), None);
    }
    assert_eq!(Bridge::default().reconcile(true, true, Some(2)), Some(1));
}
#[test]
fn session_limit_caps_hardware_brightness_instead_of_scaling_configured_active_level() {
    let mut p = Power::new(
        BacklightLevels {
            active: 1,
            dimmed: 1,
        },
        Timestamp(0),
    );
    p.limit(32768);
    assert_eq!(p.level(Timestamp(0)), 1);
}
