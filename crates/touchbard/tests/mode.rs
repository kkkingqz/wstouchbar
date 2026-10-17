use touchbar_model::Timestamp;
use touchbar_protocol::Mode;
use touchbard::machine::mode::*;
fn native() -> ModeMachine {
    let mut m = ModeMachine::new();
    let commands = m.handle(Timestamp(0), Input::Device(Some(7)));
    let [Command::ToNative { generation, .. }] = commands.as_slice() else {
        panic!("{commands:?}")
    };
    m.handle(
        Timestamp(0),
        Input::NativeSafe {
            generation: *generation,
        },
    );
    m
}
fn display(m: &mut ModeMachine, at: u64) -> u64 {
    let commands = m.handle(Timestamp(at), Input::Target(Mode::Display));
    let commands = if commands.is_empty() {
        m.advance(Timestamp(at.max(2000)))
    } else {
        commands
    };
    let [Command::ToDisplay { generation, .. }] = commands.as_slice() else {
        panic!("{commands:?}")
    };
    m.handle(
        Timestamp(at.max(2000)),
        Input::DisplayReady {
            generation: *generation,
        },
    );
    *generation
}
#[test]
fn stable_device_and_native_driver_have_independent_deadlines() {
    let mut m = native();
    assert!(
        m.handle(Timestamp(100), Input::Target(Mode::Display))
            .is_empty()
    );
    assert!(m.advance(Timestamp(1999)).is_empty());
    let cmds = m.advance(Timestamp(2000));
    let [Command::ToDisplay { generation, .. }] = cmds.as_slice() else {
        panic!("{cmds:?}")
    };
    m.handle(
        Timestamp(2001),
        Input::DisplayReady {
            generation: *generation,
        },
    );
    assert!(matches!(m.state(), State::Display { .. }));
    let cmds = m.handle(Timestamp(2100), Input::Target(Mode::Native));
    let [Command::ToNative { generation, .. }] = cmds.as_slice() else {
        panic!("{cmds:?}")
    };
    m.handle(
        Timestamp(2200),
        Input::NativeSafe {
            generation: *generation,
        },
    );
    assert_eq!(m.state(), &State::Native { safe: true });
    assert!(matches!(m.driver(), DriverState::Pending { .. }));
    assert!(matches!(
        m.handle(Timestamp(2201), Input::Target(Mode::Display))
            .as_slice(),
        [Command::ToDisplay { .. }]
    ));
}
#[test]
fn three_failures_roll_back_and_stop_until_target_changes() {
    let mut m = native();
    m.handle(Timestamp(0), Input::Target(Mode::Display));
    let mut cmds = m.advance(Timestamp(2000));
    for attempt in 1..=3 {
        let [Command::ToDisplay { generation, .. }] = cmds.as_slice() else {
            panic!("{cmds:?}")
        };
        let deadline = m.next_deadline().unwrap();
        let rollback = m.advance(deadline);
        let [
            Command::ToNative {
                generation: rollback,
                ..
            },
        ] = rollback.as_slice()
        else {
            panic!("{rollback:?}")
        };
        assert!(
            m.handle(
                deadline,
                Input::DisplayReady {
                    generation: *generation
                }
            )
            .is_empty()
        );
        let safe = m.handle(
            deadline,
            Input::NativeSafe {
                generation: *rollback,
            },
        );
        assert!(safe.is_empty());
        assert_eq!(m.failures(), attempt);
        if attempt < 3 {
            cmds = m.advance(m.next_deadline().unwrap());
        }
    }
    m.advance(Timestamp(100000));
    assert_eq!(m.state(), &State::Native { safe: true });
    assert_eq!(m.next_deadline(), None);
    m.handle(Timestamp(100000), Input::Target(Mode::Native));
    assert!(matches!(
        m.handle(Timestamp(100001), Input::Target(Mode::Display))
            .as_slice(),
        [Command::ToDisplay { .. }]
    ));
}
#[test]
fn native_safety_timeout_does_not_wait_for_hid_and_accepts_late_results() {
    let mut m = native();
    display(&mut m, 2000);
    let cmds = m.handle(Timestamp(3000), Input::Target(Mode::Native));
    let [Command::ToNative { generation, .. }] = cmds.as_slice() else {
        panic!()
    };
    m.advance(Timestamp(6000));
    assert_eq!(m.state(), &State::Native { safe: false });
    assert!(m.error().is_some());
    m.handle(
        Timestamp(6100),
        Input::NativeSafe {
            generation: *generation,
        },
    );
    assert_eq!(m.state(), &State::Native { safe: true });
    m.advance(Timestamp(16100));
    assert_eq!(m.driver(), &DriverState::Missing);
    assert_eq!(m.next_deadline(), None);
    m.handle(
        Timestamp(17000),
        Input::Driver {
            device: 7,
            mode: Some(1),
        },
    );
    assert_eq!(m.driver(), &DriverState::Ready { mode: 1 });
}
#[test]
fn reenumeration_invalidates_results_and_restarts_stability_window() {
    let mut m = native();
    let old = display(&mut m, 2000);
    assert!(matches!(
        m.handle(Timestamp(2100), Input::Device(None)).as_slice(),
        [Command::ReleaseHardware { .. }]
    ));
    m.handle(Timestamp(2200), Input::Device(Some(8)));
    assert!(
        m.handle(Timestamp(2300), Input::DisplayReady { generation: old })
            .is_empty()
    );
    assert!(m.advance(Timestamp(4199)).is_empty());
    // Native rollback is still in progress; finish it before entering Display.
    let generation = m.generation();
    m.handle(Timestamp(4199), Input::NativeSafe { generation });
    assert!(matches!(
        m.advance(Timestamp(4200)).as_slice(),
        [Command::ToDisplay { device: 8, .. }]
    ));
}
#[test]
fn idle_native_has_no_periodic_deadline() {
    let mut m = ModeMachine::new();
    assert_eq!(m.next_deadline(), None);
    for now in [0, 1, 10000, 1000000] {
        assert!(m.advance(Timestamp(now)).is_empty());
        assert_eq!(m.next_deadline(), None);
    }
}
proptest::proptest! {
    #[test]
    fn transition_policy_survives_reordered_and_stale_device_results(events in proptest::collection::vec((0u8..8,proptest::prelude::any::<u8>(),0u64..10000),1..200)) {
        let mut m=ModeMachine::new(); let mut now=0u64;
        for (kind,value,elapsed) in events {
            now+=elapsed;
            let generation=if value%2==0 {m.generation()} else {m.generation().saturating_sub(1)};
            let event=match kind {
                0=>Input::Device(if value%2==0 {Some(u64::from(value%3))} else {None}),
                1=>Input::Target(if value%2==0 {Mode::Display} else {Mode::Native}),
                2=>Input::DisplayReady{generation},
                3=>Input::NativeSafe{generation},
                4=>Input::Failed{generation,message:"test failure".into()},
                5=>Input::Driver{device:u64::from(value%3),mode:Some(value%5)},
                _=>{m.advance(Timestamp(now));continue;}
            };
            m.handle(Timestamp(now),event);
            if matches!(m.state(),State::Display{..}) {proptest::prop_assert_eq!(m.target(),Mode::Display);}
            if let Some(deadline)=m.next_deadline() {proptest::prop_assert!(deadline>Timestamp(now),"overdue deadline {deadline:?}: {m:?}");}
            proptest::prop_assert!(m.failures()<=3);
        }
    }
}
