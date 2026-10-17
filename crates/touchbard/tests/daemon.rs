use touchbar_model::{KeyPhase, SystemKey, Timestamp, TouchPhase, WatchKey};
use touchbar_protocol::{
    DaemonMessage, Mode, ModeReason, RevokedReason, SessionMessage, SwapchainHeader, XRGB8888,
};
use touchbard::{
    Command, Daemon, DisplayInfo, Event, Outgoing, Session, config::Config, machine::mode,
};
fn session(uid: u32, id: &str) -> Session {
    Session {
        uid,
        id: id.into(),
        user_display: Some(id.into()),
        active: true,
        state: "active".into(),
        class: "user".into(),
        kind: "wayland".into(),
        locked: false,
    }
}
fn info() -> DisplayInfo {
    DisplayInfo {
        header: SwapchainHeader {
            logical_width: 20,
            logical_height: 4,
            physical_width: 20,
            physical_height: 4,
            format: XRGB8888,
            pitch: 80,
            buffer_size: 320,
            buffer_count: 3,
            frame_damage: true,
        },
        caps: 1,
        width_mm100: 2000,
        height_mm100: 400,
    }
}
fn drain(d: &mut Daemon, conn: u64, now: u64) -> Vec<Outgoing> {
    let mut out = Vec::new();
    while let Some(message) = d.pending(conn).cloned() {
        out.push(message);
        d.handle(Timestamp(now), Event::Sent { connection: conn });
    }
    out
}
fn owned() -> Daemon {
    let mut d = Daemon::new(Config::default(), false, Timestamp(0)).unwrap();
    d.handle(Timestamp(0), Event::Session(Some(session(1000, "s1"))));
    d.handle(Timestamp(0), Event::Device(Some(7)));
    let generation = d.mode().generation();
    d.handle(Timestamp(0), Event::NativeSafe { generation });
    d.handle(
        Timestamp(0),
        Event::Hello {
            connection: 1,
            uid: 1000,
            version: 2,
        },
    );
    d
}
fn displaying() -> Daemon {
    let mut d = owned();
    drain(&mut d, 1, 0);
    d.advance(Timestamp(2000));
    let generation = d.mode().generation();
    d.handle(
        Timestamp(2001),
        Event::DisplayReady {
            generation,
            info: info(),
        },
    );
    drain(&mut d, 1, 2001);
    d
}
#[test]
fn native_hello_is_confirmed_without_fabricated_welcome_then_display_sends_real_info() {
    let mut d = owned();
    let native = drain(&mut d, 1, 0);
    assert_eq!(
        native,
        vec![Outgoing::Message(DaemonMessage::ModeChanged {
            mode: Mode::Native,
            reason: ModeReason::Owner
        })]
    );
    d.advance(Timestamp(2000));
    let generation = d.mode().generation();
    d.handle(
        Timestamp(2001),
        Event::DisplayReady {
            generation,
            info: info(),
        },
    );
    let display = drain(&mut d, 1, 2001);
    assert!(matches!(
        display.as_slice(),
        [
            Outgoing::Message(DaemonMessage::Welcome {
                width: 20,
                height: 4,
                ..
            }),
            Outgoing::Message(DaemonMessage::ModeChanged {
                mode: Mode::Display,
                ..
            }),
            Outgoing::Swapchain { .. }
        ]
    ));
}
#[test]
fn ownership_is_rechecked_on_every_logind_change_and_all_hello_fields() {
    let mut d = displaying();
    d.handle(Timestamp(2100), Event::Session(Some(session(1001, "s2"))));
    assert_eq!(d.owner(), None);
    assert!(matches!(d.mode().state(), mode::State::ToNative { .. }));
    let messages = drain(&mut d, 1, 2100);
    assert!(matches!(
        messages.last(),
        Some(Outgoing::Message(DaemonMessage::Revoked(
            RevokedReason::SessionInactive
        )))
    ));
    d.handle(
        Timestamp(2100),
        Event::Hello {
            connection: 2,
            uid: 1000,
            version: 2,
        },
    );
    assert_eq!(
        drain(&mut d, 2, 2100),
        vec![Outgoing::Message(DaemonMessage::Revoked(
            RevokedReason::NotActive
        ))]
    );
    d.handle(
        Timestamp(2100),
        Event::Hello {
            connection: 3,
            uid: 1001,
            version: 1,
        },
    );
    assert_eq!(
        drain(&mut d, 3, 2100),
        vec![Outgoing::Message(DaemonMessage::Revoked(
            RevokedReason::Version
        ))]
    );
}
#[test]
fn grace_keeps_only_existing_display_and_is_irrevocably_cancelled_on_lock() {
    let mut d = displaying();
    d.handle(Timestamp(2100), Event::Disconnected { connection: 1 });
    assert_eq!(d.owner(), None);
    assert_eq!(d.target(), Mode::Display);
    assert!(d.advance(Timestamp(4099)).is_empty());
    d.advance(Timestamp(4100));
    assert!(matches!(d.mode().state(), mode::State::ToNative { .. }));
    let mut d = displaying();
    d.handle(Timestamp(2100), Event::Disconnected { connection: 1 });
    let mut locked = session(1000, "s1");
    locked.locked = true;
    d.handle(Timestamp(2200), Event::Session(Some(locked)));
    d.handle(Timestamp(2201), Event::Session(Some(session(1000, "s1"))));
    assert_eq!(d.target(), Mode::Native);
    let mut d = owned();
    d.handle(Timestamp(1), Event::Disconnected { connection: 1 });
    assert_eq!(d.target(), Mode::Native);
}
#[test]
fn cancel_and_uinput_release_precede_native_or_revocation_tracking_zero_is_valid() {
    let mut d = displaying();
    d.handle(
        Timestamp(2100),
        Event::Touch {
            tracking: 0,
            phase: TouchPhase::Down,
            x: 1,
            y: 2,
        },
    );
    let touch = drain(&mut d, 1, 2100);
    assert!(
        matches!(touch.as_slice(),[Outgoing::Message(DaemonMessage::Touch{contact,..})] if contact.get()==1)
    );
    d.handle(
        Timestamp(2101),
        Event::Message {
            connection: 1,
            message: SessionMessage::Key {
                key: SystemKey::F1,
                phase: KeyPhase::Pressed,
            },
        },
    );
    let commands = d.handle(Timestamp(2102), Event::SetMode(Mode::Native));
    assert!(commands.contains(&Command::Uinput {
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    let out = drain(&mut d, 1, 2102);
    assert!(matches!(
        out.as_slice(),
        [
            Outgoing::Message(DaemonMessage::Touch {
                phase: TouchPhase::Cancel,
                ..
            }),
            Outgoing::Message(DaemonMessage::ModeChanged {
                mode: Mode::Native,
                ..
            })
        ]
    ));
    assert!(
        !d.handle(
            Timestamp(2103),
            Event::Message {
                connection: 1,
                message: SessionMessage::Key {
                    key: SystemKey::F1,
                    phase: KeyPhase::Pressed
                }
            }
        )
        .iter()
        .any(|c| matches!(
            c,
            Command::Uinput {
                phase: KeyPhase::Pressed,
                ..
            }
        ))
    );
}
#[test]
fn sleep_releases_inhibitor_on_safe_native_without_waiting_for_driver_and_after_four_seconds_on_failure()
 {
    for finish in [true, false] {
        let mut d = displaying();
        let commands = d.handle(
            Timestamp(2100),
            Event::Start {
                preparing_sleep: false,
            },
        );
        let generation = commands
            .iter()
            .find_map(|c| {
                if let Command::AcquireInhibitor { generation } = c {
                    Some(*generation)
                } else {
                    None
                }
            })
            .unwrap();
        d.handle(Timestamp(2100), Event::InhibitorAcquired { generation });
        d.handle(Timestamp(2200), Event::PrepareForSleep(true));
        let operation = d.mode().generation();
        if finish {
            let cmds = d.handle(
                Timestamp(2300),
                Event::NativeSafe {
                    generation: operation,
                },
            );
            assert!(cmds.contains(&Command::ReleaseInhibitor { generation }));
            assert!(matches!(
                d.mode().driver(),
                mode::DriverState::Pending { .. }
            ));
        } else {
            let cmds = d.advance(Timestamp(5200));
            assert!(!cmds.contains(&Command::ReleaseInhibitor { generation }));
            let cmds = d.advance(Timestamp(6200));
            assert!(cmds.contains(&Command::ReleaseInhibitor { generation }));
            assert_eq!(d.mode().state(), &mode::State::Native { safe: false });
        }
        let cmds = d.handle(Timestamp(6300), Event::PrepareForSleep(false));
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Command::AcquireInhibitor { .. }))
        );
    }
}
#[test]
fn frame_accounting_limits_inflight_and_ignores_stale_presentations() {
    let mut d = displaying();
    let generation = d.mode().generation();
    d.handle(
        Timestamp(2100),
        Event::Message {
            connection: 1,
            message: SessionMessage::FrameReady { index: 0, frame: 1 },
        },
    );
    d.handle(
        Timestamp(2101),
        Event::FramePresented {
            generation,
            connection: 1,
            index: 0,
            frame: 1,
            commit_us: 123,
        },
    );
    assert!(matches!(
        drain(&mut d, 1, 2101).as_slice(),
        [Outgoing::Message(DaemonMessage::FramePresented {
            commit_us: 123,
            ..
        })]
    ));
    d.handle(
        Timestamp(2102),
        Event::FrameReleased {
            generation,
            connection: 1,
            index: 0,
            frame: 1,
        },
    );
    drain(&mut d, 1, 2102);
    d.handle(
        Timestamp(2103),
        Event::Message {
            connection: 1,
            message: SessionMessage::FrameReady { index: 1, frame: 2 },
        },
    );
    d.handle(
        Timestamp(2104),
        Event::FramePresented {
            generation: generation - 1,
            connection: 1,
            index: 0,
            frame: 3,
            commit_us: 1,
        },
    );
    assert_eq!(d.pending(1), None);
    d.handle(
        Timestamp(2105),
        Event::Message {
            connection: 1,
            message: SessionMessage::FrameReady { index: 2, frame: 4 },
        },
    );
    assert_eq!(d.owner(), None);
}
#[test]
fn reload_preserves_lid_and_session_backlight_limit() {
    let mut d = displaying();
    d.handle(Timestamp(2100), Event::Lid(true));
    d.handle(Timestamp(2101), Event::Config(Config::default()));
    assert_eq!(d.backlight(), 0);
    d.handle(Timestamp(2102), Event::Lid(false));
    d.handle(
        Timestamp(2103),
        Event::Message {
            connection: 1,
            message: SessionMessage::Backlight { limit: 0 },
        },
    );
    d.handle(Timestamp(2104), Event::Config(Config::default()));
    assert_eq!(d.backlight(), 0);
}
#[test]
fn released_before_presented_is_ignored_and_duplicate_hello_releases_keys_before_closing() {
    let mut d = displaying();
    let generation = d.mode().generation();
    d.handle(
        Timestamp(2100),
        Event::Message {
            connection: 1,
            message: SessionMessage::FrameReady { index: 0, frame: 1 },
        },
    );
    d.handle(
        Timestamp(2101),
        Event::FrameReleased {
            generation,
            connection: 1,
            index: 0,
            frame: 1,
        },
    );
    assert!(d.pending(1).is_none());
    d.handle(
        Timestamp(2102),
        Event::Message {
            connection: 1,
            message: SessionMessage::Key {
                key: SystemKey::F1,
                phase: KeyPhase::Pressed,
            },
        },
    );
    let out = d.handle(
        Timestamp(2103),
        Event::Hello {
            connection: 1,
            uid: 1000,
            version: 2,
        },
    );
    assert!(out.contains(&Command::Uinput {
        key: SystemKey::F1,
        phase: KeyPhase::Released
    }));
    assert_eq!(d.owner(), None);
}
#[test]
fn new_connection_during_grace_gets_fresh_swapchain_without_usb_round_trip() {
    let mut d = displaying();
    let old = d.mode().generation();
    d.handle(Timestamp(2100), Event::Disconnected { connection: 1 });
    let out = d.handle(
        Timestamp(2200),
        Event::Hello {
            connection: 2,
            uid: 1000,
            version: 2,
        },
    );
    assert!(
        out.iter()
            .any(|c| matches!(c, Command::Device(mode::Command::RefreshSwapchain { .. })))
    );
    assert!(!out.iter().any(|c| matches!(
        c,
        Command::Device(mode::Command::ToNative { .. } | mode::Command::ToDisplay { .. })
    )));
    d.handle(
        Timestamp(2300),
        Event::DisplayReady {
            generation: old,
            info: info(),
        },
    );
    assert!(
        !drain(&mut d, 2, 2300)
            .iter()
            .any(|m| matches!(m, Outgoing::Swapchain { .. }))
    );
}
#[test]
fn virtual_activity_cannot_start_recovery_but_physical_fn_can_and_marker_is_explicit() {
    let mut d = displaying();
    d.handle(Timestamp(2100), Event::VirtualActivity);
    d.advance(Timestamp(20000));
    assert!(!d.recovery());
    d.handle(
        Timestamp(20001),
        Event::PhysicalKey(touchbar_bindings::KeyInput {
            code: WatchKey::Fn,
            phase: KeyPhase::Pressed,
        }),
    );
    assert!(
        !d.advance(Timestamp(28000))
            .contains(&Command::WriteRecovery(true))
    );
    assert!(
        d.advance(Timestamp(28001))
            .contains(&Command::WriteRecovery(true))
    );
    d.handle(Timestamp(28002), Event::Disconnected { connection: 1 });
    d.handle(
        Timestamp(28003),
        Event::Hello {
            connection: 2,
            uid: 1000,
            version: 2,
        },
    );
    assert!(d.recovery());
    assert!(
        d.handle(Timestamp(28004), Event::SetMode(Mode::Display))
            .contains(&Command::WriteRecovery(false))
    );
}
#[test]
fn sleeping_restart_and_shutdown_restore_autodim_and_do_not_wait_for_hid() {
    let mut d = owned();
    d.handle(
        Timestamp(1),
        Event::Start {
            preparing_sleep: true,
        },
    );
    d.advance(Timestamp(2000));
    assert_eq!(d.target(), Mode::Native);
    d.handle(Timestamp(2001), Event::PrepareForSleep(false));
    d.advance(Timestamp(2002));
    let generation = d.mode().generation();
    d.handle(
        Timestamp(2003),
        Event::DisplayReady {
            generation,
            info: info(),
        },
    );
    assert!(
        d.handle(Timestamp(2100), Event::Autodim(true))
            .contains(&Command::OwnAutodim)
    );
    let stop = d.handle(Timestamp(2200), Event::Shutdown);
    assert!(stop.contains(&Command::RestoreAutodim));
    assert!(!stop.contains(&Command::Exit));
    let generation = d.mode().generation();
    assert!(
        d.handle(Timestamp(2300), Event::NativeSafe { generation })
            .contains(&Command::Exit)
    );
    assert_eq!(d.next_deadline(), None);
}
#[test]
fn wake_contact_is_suppressed_until_up_and_new_touch_gets_fresh_id() {
    let mut d = displaying();
    d.handle(
        Timestamp(2100),
        Event::DriverTimeouts {
            dim_seconds: Some(1),
            idle_seconds: Some(1),
        },
    );
    d.advance(Timestamp(4100));
    assert_eq!(d.backlight(), 0);
    for (t, phase) in [
        (4200, TouchPhase::Down),
        (4201, TouchPhase::Move),
        (4202, TouchPhase::Up),
    ] {
        d.handle(
            Timestamp(t),
            Event::Touch {
                tracking: 0,
                phase,
                x: 1,
                y: 1,
            },
        );
        assert_eq!(d.pending(1), None);
    }
    assert_eq!(d.backlight(), 2);
    d.handle(
        Timestamp(4203),
        Event::Touch {
            tracking: 0,
            phase: TouchPhase::Down,
            x: 2,
            y: 2,
        },
    );
    assert!(
        matches!(d.pending(1),Some(Outgoing::Message(DaemonMessage::Touch{contact,..})) if contact.get()==1)
    );
}
#[test]
fn move_coalescing_keeps_edges_and_overflow_cancels_then_eventually_closes() {
    let mut d = displaying();
    d.handle(
        Timestamp(2100),
        Event::Touch {
            tracking: 0,
            phase: TouchPhase::Down,
            x: 0,
            y: 0,
        },
    );
    for x in 1..1000 {
        d.handle(
            Timestamp(2101),
            Event::Touch {
                tracking: 0,
                phase: TouchPhase::Move,
                x,
                y: 0,
            },
        );
    }
    d.handle(
        Timestamp(2102),
        Event::Touch {
            tracking: 0,
            phase: TouchPhase::Up,
            x: 1000,
            y: 0,
        },
    );
    let out = drain(&mut d, 1, 2102);
    assert!(matches!(
        out.as_slice(),
        [
            Outgoing::Message(DaemonMessage::Touch {
                phase: TouchPhase::Down,
                ..
            }),
            Outgoing::Message(DaemonMessage::Touch {
                phase: TouchPhase::Move,
                x: 999,
                ..
            }),
            Outgoing::Message(DaemonMessage::Touch {
                phase: TouchPhase::Up,
                ..
            })
        ]
    ));
    d.handle(
        Timestamp(2200),
        Event::Touch {
            tracking: 0,
            phase: TouchPhase::Down,
            x: 0,
            y: 0,
        },
    );
    for _ in 0..300 {
        d.handle(
            Timestamp(2200),
            Event::Message {
                connection: 1,
                message: SessionMessage::WatchKey {
                    key: WatchKey::Fn,
                    enabled: true,
                },
            },
        );
    }
    assert_eq!(d.owner(), None);
    assert!(d.queue_len(1) <= 290);
    let commands = d.advance(Timestamp(3200));
    assert!(commands.contains(&Command::Close { connection: 1 }));
    assert_eq!(d.pending(1), None);
}
#[test]
fn driver_bridge_is_restored_before_leaving_native_and_recovery_marker_survives_new_daemon() {
    let mut d = Daemon::new(Config::default(), true, Timestamp(0)).unwrap();
    d.handle(Timestamp(0), Event::Device(Some(7)));
    let generation = d.mode().generation();
    d.handle(Timestamp(1), Event::NativeSafe { generation });
    d.handle(
        Timestamp(2),
        Event::Driver {
            device: 7,
            mode: Some(1),
        },
    );
    assert!(
        d.handle(
            Timestamp(3),
            Event::PhysicalKey(touchbar_bindings::KeyInput {
                code: WatchKey::Fn,
                phase: KeyPhase::Pressed
            })
        )
        .contains(&Command::WriteDriverMode(2))
    );
    let out = d.handle(Timestamp(4), Event::Shutdown);
    let restore = out
        .iter()
        .position(|c| *c == Command::WriteDriverMode(1))
        .unwrap();
    let exit = out.iter().position(|c| *c == Command::Exit).unwrap();
    assert!(restore < exit);
    assert!(
        Daemon::new(Config::default(), true, Timestamp(0))
            .unwrap()
            .recovery()
    );
}
#[derive(Default)]
struct Observer {
    pressed: std::collections::BTreeMap<SystemKey, u64>,
    modes: std::collections::BTreeMap<u64, Mode>,
    contacts: std::collections::BTreeMap<u64, std::collections::BTreeSet<u32>>,
    last: std::collections::BTreeMap<u64, u32>,
    revoked: std::collections::BTreeSet<u64>,
    autodim: bool,
}
impl Observer {
    fn commands(&mut self, owner: Option<u64>, commands: &[Command]) {
        for command in commands {
            match *command {
                Command::Uinput {
                    key,
                    phase: KeyPhase::Pressed,
                } => {
                    assert!(self.pressed.insert(key, owner.unwrap()).is_none());
                }
                Command::Uinput {
                    key,
                    phase: KeyPhase::Released,
                } => {
                    self.pressed.remove(&key);
                }
                Command::Close { connection } => {
                    assert!(!self.pressed.values().any(|id| *id == connection));
                    self.contacts.remove(&connection);
                    self.modes.remove(&connection);
                }
                Command::OwnAutodim => self.autodim = true,
                Command::RestoreAutodim => self.autodim = false,
                Command::Exit => assert!(!self.autodim),
                _ => {}
            }
        }
    }
    fn message(&mut self, conn: u64, message: &Outgoing) {
        assert!(!self.revoked.contains(&conn), "T2: message after Revoked");
        match message {
            Outgoing::Message(DaemonMessage::ModeChanged { mode, .. }) => {
                if *mode == Mode::Native {
                    assert!(self.contacts.entry(conn).or_default().is_empty(), "T4");
                    assert!(!self.pressed.values().any(|id| *id == conn), "T3");
                }
                self.modes.insert(conn, *mode);
            }
            Outgoing::Message(DaemonMessage::Revoked(_)) => {
                assert!(self.contacts.entry(conn).or_default().is_empty(), "T4");
                assert!(!self.pressed.values().any(|id| *id == conn), "T3");
                self.revoked.insert(conn);
            }
            Outgoing::Message(DaemonMessage::Touch { phase, contact, .. }) => {
                assert_eq!(self.modes.get(&conn), Some(&Mode::Display), "T5");
                let id = contact.get();
                match phase {
                    TouchPhase::Down => {
                        assert!(
                            id > *self.last.entry(conn).or_default(),
                            "T4: id not growing"
                        );
                        self.last.insert(conn, id);
                        assert!(self.contacts.entry(conn).or_default().insert(id));
                    }
                    TouchPhase::Move => {
                        assert!(self.contacts.entry(conn).or_default().contains(&id))
                    }
                    TouchPhase::Up | TouchPhase::Cancel => {
                        assert!(self.contacts.entry(conn).or_default().remove(&id))
                    }
                }
            }
            _ => {}
        }
    }
}
fn edge(message: &Outgoing) -> bool {
    matches!(
        message,
        Outgoing::Message(DaemonMessage::Touch {
            phase: TouchPhase::Down | TouchPhase::Up | TouchPhase::Cancel,
            ..
        })
    )
}
proptest::proptest! {
    #[test]
    fn t1_through_t10_on_randomized_daemon_inputs(events in proptest::collection::vec((0u8..27,proptest::prelude::any::<u8>(),0u64..2000),1..160)) {
        let mut d=displaying();let mut observer=Observer::default();observer.modes.insert(1,Mode::Display);
        let mut now=2001u64;let mut connection=1u64;let mut frame=0u64;let mut inhibitor=0u64;
        for (kind,value,elapsed) in events {
            now+=elapsed;let owner=d.owner().unwrap_or(1);let generation=if value%2==0 {d.mode().generation()} else {d.mode().generation().saturating_sub(1)};
            let phase=if value%2==0 {KeyPhase::Pressed} else {KeyPhase::Released};
            let event=match kind {
                0=>Event::Session(Some(session(1000,"s1"))),
                1=>{let mut s=session(if value%2==0 {1001} else {1000},"s1");match value%6 {0=>s.locked=true,1=>s.class="greeter".into(),2=>s.state="closing".into(),3=>s.active=false,4=>s.kind="tty".into(),_=>s.user_display=Some("other".into())};Event::Session(Some(s))},
                2=>Event::Device(if value%2==0 {Some(7)} else {None}),
                3=>{connection+=1;Event::Hello{connection,uid:if value%2==0 {1000} else {1001},version:if value%3==0 {1} else {2}}},
                4=>Event::Disconnected{connection:owner},
                5=>Event::SetMode(if value%2==0 {Mode::Native} else {Mode::Display}),
                6=>Event::PhysicalKey(touchbar_bindings::KeyInput{code:if value%3==0 {WatchKey::Esc} else {WatchKey::Fn},phase}),
                7=>Event::VirtualActivity,
                8=>Event::PrepareForSleep(value%2==0),
                9=>Event::DisplayReady{generation,info:info()},
                10=>Event::NativeSafe{generation},
                11=>Event::TransitionFailed{generation,message:"random test error".into()},
                12=>Event::Driver{device:7,mode:if value%3==0 {None} else {Some(value%4)}},
                13=>Event::Touch{tracking:u32::from(value%3),phase:match value%4 {0=>TouchPhase::Down,1=>TouchPhase::Move,2=>TouchPhase::Up,_=>TouchPhase::Cancel},x:i32::from(value),y:0},
                14=>Event::Message{connection:owner,message:SessionMessage::Key{key:SystemKey::F1,phase}},
                15=>Event::Message{connection:owner,message:SessionMessage::WatchKey{key:WatchKey::Fn,enabled:value%2==0}},
                16=>{frame+=1;Event::Message{connection:owner,message:SessionMessage::FrameReady{index:u32::from(value%4),frame}}},
                17=>Event::FramePresented{generation,connection:owner,index:u32::from(value%3),frame,commit_us:100},
                18=>{
                    let conn=1+u64::from(value)%connection;
                    if let Some(message)=d.pending(conn) {observer.message(conn,message);}
                    Event::Sent{connection:conn}
                },
                19=>Event::DriverTimeouts{dim_seconds:Some(u64::from(value%3)),idle_seconds:Some(1)},
                20=>Event::Lid(value%2==0),
                21=>Event::Autodim(value%2==0),
                22=>Event::Config(Config::default()),
                23=>Event::InhibitorAcquired{generation:inhibitor},
                24=>Event::Start{preparing_sleep:value%2==0},
                26=>if value%20==0 {Event::Shutdown} else {Event::Activity},
                _=>Event::FrameReleased{generation,connection:owner,index:u32::from(value%3),frame},
            };
            let before:Vec<_>=d.queued(owner).filter(|m|edge(m)).cloned().collect();
            let removes=matches!(event,Event::Sent{..}|Event::Disconnected{..});
            let mut copy=d.clone();let expected=copy.handle(Timestamp(now),event.clone());
            let commands=d.handle(Timestamp(now),event);
            proptest::prop_assert_eq!(&commands,&expected,"determinism");
            for command in &commands {if let Command::AcquireInhibitor{generation}=command {inhibitor = *generation;}}
            observer.commands(d.owner(),&commands);
            if !removes && !commands.iter().any(|c|matches!(c,Command::Close{connection} if *connection==owner)) {
                let after:Vec<_>=d.queued(owner).filter(|m|edge(m)).cloned().collect();
                proptest::prop_assert!(after.starts_with(&before),"T6: edge removed or reordered");
            }
            proptest::prop_assert_eq!(d.check_invariants(),Ok(()));
            if commands.contains(&Command::Exit) {break;}
        }
    }
}
#[test]
fn input_waits_for_current_display_notification_to_cross_the_output_queue() {
    let mut d = owned();
    d.advance(Timestamp(2000));
    let generation = d.mode().generation();
    d.handle(
        Timestamp(2001),
        Event::DisplayReady {
            generation,
            info: info(),
        },
    );
    let press = Event::Message {
        connection: 1,
        message: SessionMessage::Key {
            key: SystemKey::F1,
            phase: KeyPhase::Pressed,
        },
    };
    assert!(
        !d.handle(Timestamp(2002), press.clone())
            .iter()
            .any(|c| matches!(
                c,
                Command::Uinput {
                    phase: KeyPhase::Pressed,
                    ..
                }
            ))
    );
    drain(&mut d, 1, 2003);
    assert!(d.handle(Timestamp(2004), press).iter().any(|c| matches!(
        c,
        Command::Uinput {
            phase: KeyPhase::Pressed,
            ..
        }
    )));
}
#[test]
fn inhibitor_failure_can_retry_on_logind_reconnect_without_idle_polling() {
    let mut d = Daemon::new(Config::default(), false, Timestamp(0)).unwrap();
    let commands = d.handle(
        Timestamp(0),
        Event::Start {
            preparing_sleep: false,
        },
    );
    let generation = commands
        .iter()
        .find_map(|c| {
            if let Command::AcquireInhibitor { generation } = c {
                Some(*generation)
            } else {
                None
            }
        })
        .unwrap();
    d.handle(
        Timestamp(1),
        Event::InhibitorFailed {
            generation,
            message: "bus disconnected".into(),
        },
    );
    assert_eq!(d.inhibitor_error(), Some("bus disconnected"));
    assert_eq!(d.next_deadline(), None);
    assert!(
        d.handle(Timestamp(1000), Event::PrepareForSleep(false))
            .iter()
            .any(|c| matches!(c,Command::AcquireInhibitor{generation:g} if *g>generation))
    );
}
#[test]
fn sleep_inhibitor_is_not_released_before_initial_device_discovery() {
    let mut d = Daemon::new(Config::default(), false, Timestamp(0)).unwrap();
    let commands = d.handle(
        Timestamp(0),
        Event::Start {
            preparing_sleep: false,
        },
    );
    let generation = commands
        .iter()
        .find_map(|c| {
            if let Command::AcquireInhibitor { generation } = c {
                Some(*generation)
            } else {
                None
            }
        })
        .unwrap();
    d.handle(Timestamp(1), Event::InhibitorAcquired { generation });
    let commands = d.handle(Timestamp(2), Event::PrepareForSleep(true));
    assert!(!commands.contains(&Command::ReleaseInhibitor { generation }));
    let commands = d.handle(Timestamp(3), Event::Device(None));
    assert!(commands.contains(&Command::ReleaseInhibitor { generation }));
}
#[test]
fn duplicate_down_after_dimming_cancels_old_contact_instead_of_turning_it_into_wake_contact() {
    let mut d = displaying();
    d.handle(
        Timestamp(2100),
        Event::Touch {
            tracking: 2,
            phase: TouchPhase::Down,
            x: 0,
            y: 0,
        },
    );
    d.handle(
        Timestamp(2100),
        Event::DriverTimeouts {
            dim_seconds: Some(0),
            idle_seconds: Some(1),
        },
    );
    d.handle(
        Timestamp(3100),
        Event::Touch {
            tracking: 2,
            phase: TouchPhase::Down,
            x: 0,
            y: 0,
        },
    );
    assert_eq!(d.owner(), None);
    assert_eq!(d.check_invariants(), Ok(()));
}
#[test]
fn shutdown_before_device_scan_waits_for_discovery_or_its_safety_budget() {
    for discovered in [true, false] {
        let mut d = Daemon::new(Config::default(), false, Timestamp(0)).unwrap();
        assert!(
            !d.handle(Timestamp(1), Event::Shutdown)
                .contains(&Command::Exit)
        );
        let commands = if discovered {
            d.handle(Timestamp(2), Event::Device(None))
        } else {
            d.advance(Timestamp(3001))
        };
        assert!(commands.contains(&Command::Exit));
    }
}
#[test]
fn displayed_buffer_is_busy_but_does_not_consume_pending_quota_in_two_buffer_swapchain() {
    let mut d = owned();
    drain(&mut d, 1, 0);
    d.advance(Timestamp(2000));
    let generation = d.mode().generation();
    let mut display = info();
    display.header.buffer_count = 2;
    d.handle(
        Timestamp(2001),
        Event::DisplayReady {
            generation,
            info: display,
        },
    );
    drain(&mut d, 1, 2001);
    d.handle(
        Timestamp(2100),
        Event::Message {
            connection: 1,
            message: SessionMessage::FrameReady { index: 0, frame: 1 },
        },
    );
    d.handle(
        Timestamp(2101),
        Event::FramePresented {
            generation,
            connection: 1,
            index: 0,
            frame: 1,
            commit_us: 10,
        },
    );
    drain(&mut d, 1, 2101);
    let commands = d.handle(
        Timestamp(2102),
        Event::Message {
            connection: 1,
            message: SessionMessage::FrameReady { index: 1, frame: 2 },
        },
    );
    assert!(commands.iter().any(|c| matches!(
        c,
        Command::Present {
            index: 1,
            frame: 2,
            ..
        }
    )));
    assert_eq!(d.owner(), Some(1));
    assert_eq!(d.check_invariants(), Ok(()));
}

#[test]
fn only_one_unconfirmed_frame_even_with_three_buffers() {
    let mut d = displaying();
    d.handle(
        Timestamp(2100),
        Event::Message {
            connection: 1,
            message: SessionMessage::FrameReady { index: 0, frame: 1 },
        },
    );
    d.handle(
        Timestamp(2101),
        Event::Message {
            connection: 1,
            message: SessionMessage::FrameReady { index: 1, frame: 2 },
        },
    );
    assert_eq!(d.owner(), None);
}
#[test]
fn lost_acquired_inhibitor_retries_after_logind_reconnect_and_ignores_old_failure() {
    let mut d = Daemon::new(Config::default(), false, Timestamp(0)).unwrap();
    let gen1 = d
        .handle(
            Timestamp(0),
            Event::Start {
                preparing_sleep: false,
            },
        )
        .into_iter()
        .find_map(|c| {
            if let Command::AcquireInhibitor { generation } = c {
                Some(generation)
            } else {
                None
            }
        })
        .unwrap();
    d.handle(Timestamp(1), Event::InhibitorAcquired { generation: gen1 });
    d.handle(
        Timestamp(2),
        Event::InhibitorFailed {
            generation: gen1,
            message: "logind disconnected".into(),
        },
    );
    let gen2 = d
        .handle(Timestamp(1000), Event::PrepareForSleep(false))
        .into_iter()
        .find_map(|c| {
            if let Command::AcquireInhibitor { generation } = c {
                Some(generation)
            } else {
                None
            }
        })
        .expect("a lost acquired FD must be reacquired after reconnect");
    assert!(gen2 > gen1);
    d.handle(
        Timestamp(1001),
        Event::InhibitorAcquired { generation: gen2 },
    );
    d.handle(
        Timestamp(1002),
        Event::InhibitorFailed {
            generation: gen1,
            message: "stale loss".into(),
        },
    );
    assert_eq!(d.inhibitor_error(), None);
    assert!(
        !d.handle(Timestamp(1003), Event::PrepareForSleep(false))
            .iter()
            .any(|c| matches!(c, Command::AcquireInhibitor { .. }))
    );
}
