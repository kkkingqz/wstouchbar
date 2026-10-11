use proptest::prelude::*;
use std::collections::BTreeSet;
use touchbar_bindings::*;
use touchbar_model::{Condition, FactValue, Facts, KeyPhase, Timestamp, WatchKey};

fn engine() -> BindingEngine<u8> {
    BindingEngine::new(vec![
        Binding {
            id: "fn".into(),
            trigger: Trigger::Key(KeyTrigger {
                keys: vec![WatchKey::Fn],
                gesture: KeyGesture::WhileHeld,
            }),
            when: None,
            action: 1,
        },
        Binding {
            id: "chord".into(),
            trigger: Trigger::Key(KeyTrigger {
                keys: vec![WatchKey::Fn, WatchKey::Esc],
                gesture: KeyGesture::WhileHeld,
            }),
            when: Some(Condition::Exists("enabled".into())),
            action: 2,
        },
    ])
    .unwrap()
}
proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]
    #[test]
    fn every_began_has_exactly_one_ended(events in prop::collection::vec((0u16..1000, 0u8..3, any::<bool>(), 0u8..10), 0..150)) {
        let mut e = engine();
        let mut active = BTreeSet::new();
        let mut now = Timestamp(0);
        let mut output = vec![];
        for (delta, code, pressed, op) in events {
            now = now.after(u64::from(delta));
            let keys = [WatchKey::Fn, WatchKey::Esc, WatchKey::LeftShift];
            output.extend(match op {
                0 => e.reset(),
                1 => e.set_facts(now, Facts::new()),
                2 => e.set_facts(now, Facts::from([("enabled".into(), FactValue::Boolean(true))])),
                _ => e.handle(now, KeyInput { code: keys[usize::from(code)], phase: if pressed { KeyPhase::Pressed } else { KeyPhase::Released } }),
            });
        }
        for code in [WatchKey::Fn, WatchKey::Esc, WatchKey::LeftShift] { output.extend(e.handle(now, KeyInput { code, phase: KeyPhase::Released })); }
        output.extend(e.advance(now.after(100_000)));
        for event in output {
            match event {
                BindingEvent::Began { id, .. } => prop_assert!(active.insert(id)),
                BindingEvent::Ended { id, .. } => prop_assert!(active.remove(&id)),
                BindingEvent::Fired { .. } => prop_assert!(false, "held binding fired"),
            }
        }
        prop_assert!(active.is_empty());
        prop_assert!(e.reset().is_empty());
        prop_assert_eq!(e.next_deadline(), None);
    }
}
