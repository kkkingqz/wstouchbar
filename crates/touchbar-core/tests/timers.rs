use touchbar_core::{TimerChange, TimerRegistry};
use touchbar_model::{TimerId, Timestamp};
#[test]
fn earlier_deadline_replaces_host_timer_and_cancellation_restores_it() {
    let mut t = TimerRegistry::default();
    let a = t.schedule(Timestamp(100), "a");
    assert_eq!(
        t.notify(),
        vec![TimerChange::Set {
            id: a,
            at: Timestamp(100)
        }]
    );
    let b = t.schedule(Timestamp(50), "b");
    assert_eq!(
        t.notify(),
        vec![
            TimerChange::Clear { id: a },
            TimerChange::Set {
                id: b,
                at: Timestamp(50)
            }
        ]
    );
    assert_eq!(t.cancel(b), Some("b"));
    assert_eq!(
        t.notify(),
        vec![
            TimerChange::Clear { id: b },
            TimerChange::Set {
                id: a,
                at: Timestamp(100)
            }
        ]
    );
    assert!(t.notify().is_empty());
}
#[test]
fn stale_timer_does_not_dispatch_a_replacement() {
    let mut t = TimerRegistry::default();
    let old = t.schedule(Timestamp(50), "old");
    t.notify();
    t.cancel(old);
    let new = t.schedule(Timestamp(60), "new");
    t.notify();
    assert!(t.take_due(Timestamp(100), old).is_empty());
    assert_eq!(t.take_due(Timestamp(100), new), vec![(new, "new")]);
    assert!(t.notify().is_empty());
}
#[test]
fn late_wakeup_dispatches_due_jobs_in_deadline_and_id_order() {
    let mut t = TimerRegistry::default();
    let a = t.schedule(Timestamp(50), "a");
    let b = t.schedule(Timestamp(20), "b");
    let c = t.schedule(Timestamp(20), "c");
    let d = t.schedule(Timestamp(200), "d");
    t.notify();
    assert_eq!(
        t.take_due(Timestamp(100), b),
        vec![(b, "b"), (c, "c"), (a, "a")]
    );
    assert_eq!(
        t.notify(),
        vec![TimerChange::Set {
            id: d,
            at: Timestamp(200)
        }]
    );
}
#[test]
fn early_wakeup_rearms_same_deadline_and_does_not_fire() {
    let mut t = TimerRegistry::default();
    let id = t.schedule(Timestamp(100), "job");
    t.notify();
    assert!(t.take_due(Timestamp(99), id).is_empty());
    assert_eq!(
        t.notify(),
        vec![TimerChange::Set {
            id,
            at: Timestamp(100)
        }]
    );
    assert_eq!(t.take_due(Timestamp(100), id), vec![(id, "job")]);
    assert!(t.take_due(Timestamp(101), id).is_empty());
}
#[test]
fn cancelling_last_timer_clears_host_timer() {
    let mut t = TimerRegistry::default();
    let id = t.schedule(Timestamp(100), "job");
    t.notify();
    t.cancel(id);
    assert_eq!(t.notify(), vec![TimerChange::Clear { id }]);
    assert_eq!(t.cancel(TimerId(0)), None);
}
