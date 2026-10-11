use touchbar_bindings::*;
use touchbar_model::{Condition, FactValue, Facts, KeyPhase, Timestamp, WatchKey};

fn key(code: WatchKey, phase: KeyPhase) -> KeyInput {
    KeyInput { code, phase }
}
fn trigger(gesture: KeyGesture) -> Trigger {
    Trigger::Key(KeyTrigger {
        keys: vec![WatchKey::Fn],
        gesture,
    })
}
fn binding(id: &str, trigger: Trigger) -> Binding<String> {
    Binding {
        id: id.into(),
        trigger,
        when: None,
        action: id.into(),
    }
}
fn engine(gesture: KeyGesture) -> BindingEngine<String> {
    BindingEngine::new(vec![binding("test", trigger(gesture))]).unwrap()
}
fn fired(id: &str) -> BindingEvent<String> {
    BindingEvent::Fired {
        id: id.into(),
        action: id.into(),
    }
}
fn began(id: &str) -> BindingEvent<String> {
    BindingEvent::Began {
        id: id.into(),
        action: id.into(),
    }
}
fn ended(id: &str) -> BindingEvent<String> {
    BindingEvent::Ended {
        id: id.into(),
        action: id.into(),
    }
}

#[test]
fn recovery_hold_fires_once_and_release_rearms() {
    let mut e = engine(KeyGesture::Hold { ms: 8000 });
    assert!(
        e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed))
            .is_empty()
    );
    assert_eq!(e.next_deadline(), Some(Timestamp(8000)));
    assert!(e.advance(Timestamp(7999)).is_empty());
    assert_eq!(e.advance(Timestamp(8000)), vec![fired("test")]);
    assert!(e.advance(Timestamp(16000)).is_empty());
    assert_eq!(e.next_deadline(), None);
    e.handle(Timestamp(16001), key(WatchKey::Fn, KeyPhase::Released));
    e.handle(Timestamp(16002), key(WatchKey::Fn, KeyPhase::Pressed));
    assert_eq!(e.advance(Timestamp(24002)), vec![fired("test")]);
}
#[test]
fn interrupted_recovery_hold_does_not_fire() {
    let mut e = engine(KeyGesture::Hold { ms: 8000 });
    e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed));
    e.handle(Timestamp(7999), key(WatchKey::Fn, KeyPhase::Released));
    assert!(e.advance(Timestamp(8001)).is_empty());
}
#[test]
fn press_release_and_while_held_ignore_duplicate_key_events() {
    for (gesture, down, up) in [
        (KeyGesture::Press, vec![fired("test")], vec![]),
        (KeyGesture::Release, vec![], vec![fired("test")]),
        (
            KeyGesture::WhileHeld,
            vec![began("test")],
            vec![ended("test")],
        ),
    ] {
        let mut e = engine(gesture);
        assert_eq!(
            e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed)),
            down
        );
        assert!(
            e.handle(Timestamp(1), key(WatchKey::Fn, KeyPhase::Pressed))
                .is_empty()
        );
        assert_eq!(
            e.handle(Timestamp(2), key(WatchKey::Fn, KeyPhase::Released)),
            up
        );
        assert!(
            e.handle(Timestamp(3), key(WatchKey::Fn, KeyPhase::Released))
                .is_empty()
        );
    }
}
#[test]
fn tap_uses_strict_duration_boundary() {
    for (up, want) in [(299, vec![fired("test")]), (300, vec![]), (301, vec![])] {
        let mut e = engine(KeyGesture::Tap { ms: 300 });
        e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed));
        assert_eq!(
            e.handle(Timestamp(up), key(WatchKey::Fn, KeyPhase::Released)),
            want
        );
        assert_eq!(e.next_deadline(), None);
    }
}
#[test]
fn another_key_invalidates_tap_even_if_released_before_up() {
    let mut e = engine(KeyGesture::Tap { ms: 300 });
    e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed));
    e.handle(Timestamp(1), key(WatchKey::Esc, KeyPhase::Pressed));
    e.handle(Timestamp(2), key(WatchKey::Esc, KeyPhase::Released));
    assert!(
        e.handle(Timestamp(3), key(WatchKey::Fn, KeyPhase::Released))
            .is_empty()
    );
}
#[test]
fn hold_boundary_is_inclusive_and_timer_precedes_release() {
    for (up, want) in [
        (99, vec![]),
        (100, vec![fired("test")]),
        (101, vec![fired("test")]),
    ] {
        let mut e = engine(KeyGesture::Hold { ms: 100 });
        e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed));
        assert_eq!(
            e.handle(Timestamp(up), key(WatchKey::Fn, KeyPhase::Released)),
            want
        );
    }
}
#[test]
fn chord_begins_on_last_key_and_ends_on_first_release() {
    for keys in [[WatchKey::Fn, WatchKey::Esc], [WatchKey::Esc, WatchKey::Fn]] {
        let mut e = BindingEngine::new(vec![binding(
            "chord",
            Trigger::Key(KeyTrigger {
                keys: vec![WatchKey::Fn, WatchKey::Esc],
                gesture: KeyGesture::WhileHeld,
            }),
        )])
        .unwrap();
        assert!(
            e.handle(Timestamp(0), key(keys[0], KeyPhase::Pressed))
                .is_empty()
        );
        assert_eq!(
            e.handle(Timestamp(10), key(keys[1], KeyPhase::Pressed)),
            vec![began("chord")]
        );
        assert_eq!(
            e.handle(Timestamp(20), key(keys[0], KeyPhase::Released)),
            vec![ended("chord")]
        );
        assert!(
            e.handle(Timestamp(30), key(keys[1], KeyPhase::Released))
                .is_empty()
        );
    }
}
#[test]
fn chord_hold_counts_from_last_press() {
    let mut e = BindingEngine::new(vec![binding(
        "chord",
        Trigger::Key(KeyTrigger {
            keys: vec![WatchKey::Fn, WatchKey::Esc],
            gesture: KeyGesture::Hold { ms: 100 },
        }),
    )])
    .unwrap();
    e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed));
    e.handle(Timestamp(50), key(WatchKey::Esc, KeyPhase::Pressed));
    assert_eq!(e.next_deadline(), Some(Timestamp(150)));
    assert!(e.advance(Timestamp(149)).is_empty());
    assert_eq!(e.advance(Timestamp(150)), vec![fired("chord")]);
}
#[test]
fn multitap_window_expires_without_firing_at_boundary() {
    for (up, want) in [(299, vec![fired("test")]), (300, vec![]), (301, vec![])] {
        let mut e = engine(KeyGesture::MultiTap {
            count: 2,
            within_ms: 300,
            tap_ms: 300,
        });
        e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed));
        e.handle(Timestamp(10), key(WatchKey::Fn, KeyPhase::Released));
        assert_eq!(e.next_deadline(), Some(Timestamp(300)));
        e.handle(Timestamp(up - 1), key(WatchKey::Fn, KeyPhase::Pressed));
        assert_eq!(
            e.handle(Timestamp(up), key(WatchKey::Fn, KeyPhase::Released)),
            want
        );
    }
}
fn sequence(steps: Vec<KeyTrigger>, ms: u64) -> BindingEngine<String> {
    BindingEngine::new(vec![binding(
        "seq",
        Trigger::Sequence {
            steps,
            within_ms: ms,
        },
    )])
    .unwrap()
}
#[test]
fn sequence_taps_respect_window_and_single_event_is_one_step() {
    for (up, want) in [(109, vec![fired("seq")]), (110, vec![]), (111, vec![])] {
        let mut e = sequence(
            vec![
                KeyTrigger {
                    keys: vec![WatchKey::Fn],
                    gesture: KeyGesture::Tap { ms: 300 },
                },
                KeyTrigger {
                    keys: vec![WatchKey::Esc],
                    gesture: KeyGesture::Tap { ms: 300 },
                },
            ],
            100,
        );
        e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed));
        assert!(
            e.handle(Timestamp(10), key(WatchKey::Fn, KeyPhase::Released))
                .is_empty()
        );
        e.handle(Timestamp(up - 1), key(WatchKey::Esc, KeyPhase::Pressed));
        assert_eq!(
            e.handle(Timestamp(up), key(WatchKey::Esc, KeyPhase::Released)),
            want
        );
    }
    let step = KeyTrigger {
        keys: vec![WatchKey::Fn],
        gesture: KeyGesture::Press,
    };
    let mut e = sequence(vec![step.clone(), step], 100);
    assert!(
        e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed))
            .is_empty()
    );
    e.handle(Timestamp(1), key(WatchKey::Fn, KeyPhase::Released));
    assert_eq!(
        e.handle(Timestamp(2), key(WatchKey::Fn, KeyPhase::Pressed)),
        vec![fired("seq")]
    );
}
#[test]
fn sequence_can_match_press_then_release_of_same_key() {
    let mut e = sequence(
        vec![
            KeyTrigger {
                keys: vec![WatchKey::Fn],
                gesture: KeyGesture::Press,
            },
            KeyTrigger {
                keys: vec![WatchKey::Fn],
                gesture: KeyGesture::Release,
            },
        ],
        100,
    );
    e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed));
    assert_eq!(
        e.handle(Timestamp(1), key(WatchKey::Fn, KeyPhase::Released)),
        vec![fired("seq")]
    );
}
#[test]
fn multiple_bindings_on_same_key_are_independent() {
    let mut e = BindingEngine::new(vec![
        binding("layer", trigger(KeyGesture::WhileHeld)),
        binding("recovery", trigger(KeyGesture::Hold { ms: 8000 })),
    ])
    .unwrap();
    assert_eq!(
        e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed)),
        vec![began("layer")]
    );
    assert_eq!(e.advance(Timestamp(8000)), vec![fired("recovery")]);
    assert_eq!(
        e.handle(Timestamp(8001), key(WatchKey::Fn, KeyPhase::Released)),
        vec![ended("layer")]
    );
}
#[test]
fn reset_ends_held_binding_and_clears_deadlines() {
    let mut e = BindingEngine::new(vec![
        binding("held", trigger(KeyGesture::WhileHeld)),
        binding("hold", trigger(KeyGesture::Hold { ms: 100 })),
    ])
    .unwrap();
    e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed));
    assert_eq!(e.reset(), vec![ended("held")]);
    assert_eq!(e.next_deadline(), None);
    assert!(e.advance(Timestamp(100)).is_empty());
    assert!(e.reset().is_empty());
}
#[test]
fn condition_false_ends_active_hold_and_true_waits_for_new_press() {
    let mut b = binding("test", trigger(KeyGesture::WhileHeld));
    b.when = Some(Condition::Eq {
        name: "enabled".into(),
        value: FactValue::Boolean(true),
    });
    let mut e = BindingEngine::new(vec![b]).unwrap();
    let yes = Facts::from([("enabled".into(), FactValue::Boolean(true))]);
    e.set_facts(Timestamp(0), yes.clone());
    assert_eq!(
        e.handle(Timestamp(1), key(WatchKey::Fn, KeyPhase::Pressed)),
        vec![began("test")]
    );
    assert_eq!(e.set_facts(Timestamp(2), Facts::new()), vec![ended("test")]);
    assert!(e.set_facts(Timestamp(3), yes).is_empty());
    assert!(
        e.handle(Timestamp(4), key(WatchKey::Fn, KeyPhase::Pressed))
            .is_empty()
    );
    assert!(
        e.handle(Timestamp(5), key(WatchKey::Fn, KeyPhase::Released))
            .is_empty()
    );
    assert_eq!(
        e.handle(Timestamp(6), key(WatchKey::Fn, KeyPhase::Pressed)),
        vec![began("test")]
    );
}
#[test]
fn condition_false_cancels_pending_hold_even_at_deadline() {
    let mut b = binding("test", trigger(KeyGesture::Hold { ms: 100 }));
    b.when = Some(Condition::Exists("enabled".into()));
    let mut e = BindingEngine::new(vec![b]).unwrap();
    e.set_facts(
        Timestamp(0),
        Facts::from([("enabled".into(), FactValue::Boolean(true))]),
    );
    e.handle(Timestamp(1), key(WatchKey::Fn, KeyPhase::Pressed));
    assert!(e.set_facts(Timestamp(101), Facts::new()).is_empty());
    assert_eq!(e.next_deadline(), None);
}
#[test]
fn checkpoint_of_armed_sequence_preserves_continuation() {
    let mut e = sequence(
        vec![
            KeyTrigger {
                keys: vec![WatchKey::Fn],
                gesture: KeyGesture::Press,
            },
            KeyTrigger {
                keys: vec![WatchKey::Esc],
                gesture: KeyGesture::Hold { ms: 100 },
            },
        ],
        500,
    );
    e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed));
    e.handle(Timestamp(10), key(WatchKey::Esc, KeyPhase::Pressed));
    let mut restored: BindingEngine<String> =
        serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
    assert_eq!(e.advance(Timestamp(200)), vec![fired("seq")]);
    assert_eq!(restored.advance(Timestamp(200)), vec![fired("seq")]);
    assert_eq!(restored.next_deadline(), e.next_deadline());
}
#[test]
fn conflicts_normalize_chord_order_but_allow_distinct_conditions() {
    let a = binding(
        "a",
        Trigger::Key(KeyTrigger {
            keys: vec![WatchKey::Fn, WatchKey::Esc],
            gesture: KeyGesture::Press,
        }),
    );
    let b = binding(
        "b",
        Trigger::Key(KeyTrigger {
            keys: vec![WatchKey::Esc, WatchKey::Fn],
            gesture: KeyGesture::Press,
        }),
    );
    assert!(BindingEngine::new(vec![a.clone(), b.clone()]).is_err());
    let mut b = b;
    b.when = Some(Condition::Exists("enabled".into()));
    assert!(BindingEngine::new(vec![a, b]).is_ok());
}
#[test]
fn invalid_trigger_and_duplicate_ids_are_rejected() {
    let a = binding("a", trigger(KeyGesture::WhileHeld));
    assert!(BindingEngine::new(vec![a.clone(), a]).is_err());
    let bad = binding(
        "bad",
        Trigger::Sequence {
            steps: vec![KeyTrigger {
                keys: vec![WatchKey::Fn],
                gesture: KeyGesture::WhileHeld,
            }],
            within_ms: 100,
        },
    );
    assert!(BindingEngine::new(vec![bad]).is_err());
}
#[test]
fn chord_tap_allows_only_its_own_keys() {
    for extra in [false, true] {
        let mut e = BindingEngine::new(vec![binding(
            "chord",
            Trigger::Key(KeyTrigger {
                keys: vec![WatchKey::Fn, WatchKey::Esc],
                gesture: KeyGesture::Tap { ms: 100 },
            }),
        )])
        .unwrap();
        e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed));
        e.handle(Timestamp(10), key(WatchKey::Esc, KeyPhase::Pressed));
        if extra {
            e.handle(Timestamp(11), key(WatchKey::LeftShift, KeyPhase::Pressed));
        }
        assert_eq!(
            e.handle(Timestamp(12), key(WatchKey::Fn, KeyPhase::Released)),
            if extra { vec![] } else { vec![fired("chord")] }
        );
    }
}
#[test]
fn chord_rearmed_after_condition_uses_a_new_last_key_press() {
    let mut b = binding(
        "test",
        Trigger::Key(KeyTrigger {
            keys: vec![WatchKey::Fn, WatchKey::Esc],
            gesture: KeyGesture::WhileHeld,
        }),
    );
    b.when = Some(Condition::Exists("enabled".into()));
    let mut e = BindingEngine::new(vec![b]).unwrap();
    e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed));
    assert!(
        e.set_facts(
            Timestamp(1),
            Facts::from([("enabled".into(), FactValue::Boolean(true))])
        )
        .is_empty()
    );
    assert_eq!(
        e.handle(Timestamp(2), key(WatchKey::Esc, KeyPhase::Pressed)),
        vec![began("test")]
    );
}
#[test]
fn delayed_timer_wakeup_delivers_hold_before_later_sequence_timeout() {
    let mut e = sequence(
        vec![
            KeyTrigger {
                keys: vec![WatchKey::Fn],
                gesture: KeyGesture::Press,
            },
            KeyTrigger {
                keys: vec![WatchKey::Esc],
                gesture: KeyGesture::Hold { ms: 100 },
            },
        ],
        500,
    );
    e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed));
    e.handle(Timestamp(10), key(WatchKey::Esc, KeyPhase::Pressed));
    assert_eq!(e.advance(Timestamp(1000)), vec![fired("seq")]);
}
#[test]
fn single_step_sequence_fires_once() {
    let mut e = sequence(
        vec![KeyTrigger {
            keys: vec![WatchKey::Fn],
            gesture: KeyGesture::Press,
        }],
        100,
    );
    assert_eq!(
        e.handle(Timestamp(0), key(WatchKey::Fn, KeyPhase::Pressed)),
        vec![fired("seq")]
    );
    assert!(e.advance(Timestamp(100)).is_empty());
}
#[test]
fn condition_false_discards_sequence_progress() {
    let mut b = binding(
        "seq",
        Trigger::Sequence {
            steps: vec![
                KeyTrigger {
                    keys: vec![WatchKey::Fn],
                    gesture: KeyGesture::Press,
                },
                KeyTrigger {
                    keys: vec![WatchKey::Esc],
                    gesture: KeyGesture::Press,
                },
            ],
            within_ms: 100,
        },
    );
    b.when = Some(Condition::Exists("enabled".into()));
    let mut e = BindingEngine::new(vec![b]).unwrap();
    let yes = Facts::from([("enabled".into(), FactValue::Boolean(true))]);
    e.set_facts(Timestamp(0), yes.clone());
    e.handle(Timestamp(1), key(WatchKey::Fn, KeyPhase::Pressed));
    e.set_facts(Timestamp(2), Facts::new());
    e.set_facts(Timestamp(3), yes);
    assert!(
        e.handle(Timestamp(4), key(WatchKey::Esc, KeyPhase::Pressed))
            .is_empty()
    );
    assert_eq!(e.next_deadline(), None);
}
#[test]
fn condition_false_discards_multitap_progress() {
    let mut b = binding(
        "test",
        trigger(KeyGesture::MultiTap {
            count: 2,
            within_ms: 100,
            tap_ms: 100,
        }),
    );
    b.when = Some(Condition::Exists("enabled".into()));
    let mut e = BindingEngine::new(vec![b]).unwrap();
    let yes = Facts::from([("enabled".into(), FactValue::Boolean(true))]);
    e.set_facts(Timestamp(0), yes.clone());
    e.handle(Timestamp(1), key(WatchKey::Fn, KeyPhase::Pressed));
    e.handle(Timestamp(2), key(WatchKey::Fn, KeyPhase::Released));
    e.set_facts(Timestamp(3), Facts::new());
    e.set_facts(Timestamp(4), yes);
    e.handle(Timestamp(5), key(WatchKey::Fn, KeyPhase::Pressed));
    assert!(
        e.handle(Timestamp(6), key(WatchKey::Fn, KeyPhase::Released))
            .is_empty()
    );
    e.handle(Timestamp(7), key(WatchKey::Fn, KeyPhase::Pressed));
    assert_eq!(
        e.handle(Timestamp(8), key(WatchKey::Fn, KeyPhase::Released)),
        vec![fired("test")]
    );
}
