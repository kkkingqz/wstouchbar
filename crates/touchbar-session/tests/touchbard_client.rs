use std::time::Duration;
use touchbar_model::{Generation, KeyPhase, SurfaceId, SystemKey, Timestamp, WatchKey};
use touchbar_protocol::{self as wire, Mode, ModeReason, SessionMessage as In};
use touchbar_session::touchbard::{Client, ClientEvent};
use touchbar_testkit::{FakeTouchbarDisplay, FakeTouchbard, FakeTouchbardEvent as Peer};
fn display() -> FakeTouchbarDisplay {
    FakeTouchbarDisplay {
        header: wire::SwapchainHeader {
            logical_width: 400,
            logical_height: 60,
            physical_width: 60,
            physical_height: 400,
            format: wire::XRGB8888,
            pitch: 240,
            buffer_size: 96000,
            buffer_count: 3,
            frame_damage: true,
        },
        width_mm100: 20000,
        height_mm100: 3000,
    }
}
async fn connected() -> (FakeTouchbard, Client) {
    let (mut fake, fd) = FakeTouchbard::pair(display()).unwrap();
    let mut client = Client::from_fd(fd, SurfaceId(20)).unwrap();
    assert!(client.poll(Timestamp(0)).unwrap().is_none());
    assert!(matches!(
        fake.next_event().await.unwrap(),
        Peer::Received(In::Hello { version: 2 })
    ));
    assert!(client.poll(Timestamp(0)).unwrap().is_none());
    assert_eq!(client.status().mode, Some(Mode::Native));
    fake.set_mode(Mode::Display, ModeReason::Owner).unwrap();
    (fake, client)
}
fn drain(client: &mut Client, now: u64) -> Vec<ClientEvent> {
    let mut events = Vec::new();
    for _ in 0..32 {
        if let Some(e) = client.poll(Timestamp(now)).unwrap() {
            events.push(e)
        }
    }
    events
}
#[test]
fn touchbar_appears_with_welcome_size_and_delayed_native_has_no_surface() {
    futures_lite::future::block_on(async {
        let (_fake, mut client) = connected().await;
        let events = drain(&mut client, 0);
        let ClientEvent::Welcome(info) = &events[0] else {
            panic!("welcome first: {events:?}")
        };
        assert_eq!((info.width_px, info.height_px), (400, 60));
        assert_eq!(info.px_per_mm, 2.0);
        assert_eq!(info.name.as_str(), "touchbar");
        assert!(
            info.caps.modes
                && info.caps.backlight
                && info.caps.watch_keys
                && info.caps.inject_keys
                && info.caps.damage
        );
        let slot = client.handle().prepare_frame().unwrap().unwrap();
        assert_eq!(slot.header, display().header);
        assert_eq!(slot.buffers.unwrap().len(), 3);
        assert!(slot.force_full);
    });
}
#[test]
fn flow_control_and_frame_done_after_presented_use_separate_release_ownership() {
    futures_lite::future::block_on(async {
        let (mut fake, mut client) = connected().await;
        drain(&mut client, 0);
        fake.set_delays(Duration::ZERO, Duration::ZERO);
        let h = client.handle();
        let slot = h.prepare_frame().unwrap().unwrap();
        h.submit(Generation(7), slot.index, &[]).unwrap();
        drain(&mut client, 10);
        assert!(h.prepare_frame().unwrap().is_none());
        assert!(matches!(
            fake.next_event().await.unwrap(),
            Peer::Received(In::FrameReady { frame: 1, .. })
        ));
        assert!(matches!(
            fake.next_event().await.unwrap(),
            Peer::Presented { frame: 1, .. }
        ));
        let events = drain(&mut client, 25);
        assert!(events.iter().any(|e| matches!(
            e,
            ClientEvent::Surface(touchbar_session::surface::SurfaceEvent::Presented {
                surface: SurfaceId(20),
                generation: Generation(7)
            })
        )));
        assert_ne!(
            h.prepare_frame().unwrap().unwrap().index,
            slot.index,
            "presented buffer is still held"
        );
        assert_eq!(client.status().commit_us.samples, 1);
        assert_eq!(client.status().ack_ms.p50, Some(15.0));
    });
}
#[test]
fn watch_keys_send_differences_and_native_rejects_new_presses() {
    futures_lite::future::block_on(async {
        let (mut fake, mut client) = connected().await;
        drain(&mut client, 0);
        let h = client.handle();
        h.watch_keys(&[WatchKey::Fn, WatchKey::LeftShift]).unwrap();
        h.watch_keys(&[WatchKey::Fn]).unwrap();
        drain(&mut client, 0);
        for _ in 0..3 {
            fake.next_event().await.unwrap();
        }
        let messages = fake.take_received();
        assert_eq!(
            messages
                .iter()
                .filter(|m| matches!(
                    m,
                    In::WatchKey {
                        key: WatchKey::Fn,
                        enabled: true
                    }
                ))
                .count(),
            1
        );
        assert!(messages.contains(&In::WatchKey {
            key: WatchKey::LeftShift,
            enabled: false
        }));
        fake.set_mode(Mode::Native, ModeReason::Recovery).unwrap();
        drain(&mut client, 20);
        assert!(h.send_key(SystemKey::F5, KeyPhase::Pressed).is_err());
        assert!(h.prepare_frame().unwrap().is_none());
        h.send_key(SystemKey::F5, KeyPhase::Released).unwrap();
    });
}
#[test]
fn malformed_acknowledgement_closes_transport_and_rejects_further_outputs() {
    use rustix::net::{AddressFamily, SocketFlags, SocketType, socketpair};
    let (peer, fd) = socketpair(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let mut client = Client::from_fd(fd, SurfaceId(20)).unwrap();
    client.poll(Timestamp(0)).unwrap();
    wire::recv_session(&peer).unwrap();
    wire::send_daemon(
        &peer,
        wire::DaemonMessage::Welcome {
            caps: 1,
            width: 400,
            height: 60,
            width_mm100: 20000,
            height_mm100: 3000,
        },
    )
    .unwrap();
    let fds = (0..3)
        .map(|_| {
            let fd = rustix::fs::memfd_create(
                "bad-ack-fixture",
                rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
            )
            .unwrap();
            rustix::fs::ftruncate(&fd, 96000).unwrap();
            rustix::fs::fcntl_add_seals(
                &fd,
                rustix::fs::SealFlags::GROW | rustix::fs::SealFlags::SHRINK,
            )
            .unwrap();
            fd
        })
        .collect::<Vec<_>>();
    wire::send_swapchain(&peer, display().header, &fds).unwrap();
    wire::send_daemon(
        &peer,
        wire::DaemonMessage::ModeChanged {
            mode: Mode::Display,
            reason: ModeReason::Owner,
        },
    )
    .unwrap();
    drain(&mut client, 0);
    wire::send_daemon(
        &peer,
        wire::DaemonMessage::BufferReleased { index: 0, frame: 1 },
    )
    .unwrap();
    assert!(client.poll(Timestamp(1)).is_err());
    assert!(
        !client.status().connected,
        "malformed daemon response must close the client"
    );
    assert!(
        client
            .handle()
            .send_key(SystemKey::F5, KeyPhase::Pressed)
            .is_err()
    );
    assert_eq!(
        wire::recv_session(&peer).unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
}
#[test]
fn backlight_is_normalized_and_output_backpressure_is_a_local_disconnect() {
    futures_lite::future::block_on(async {
        let (mut fake, mut client) = connected().await;
        drain(&mut client, 0);
        let h = client.handle();
        for bad in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert!(h.backlight(bad).is_err());
        }
        for value in [0.0, 0.5, 1.0] {
            h.backlight(value).unwrap();
        }
        drain(&mut client, 0);
        for _ in 0..3 {
            fake.next_event().await.unwrap();
        }
        let messages = fake.take_received();
        for limit in [0, 32768, 65535] {
            assert!(messages.contains(&In::Backlight { limit }));
        }
        let mut rejected = false;
        for _ in 0..300 {
            if h.send_key(SystemKey::F5, KeyPhase::Released).is_err() {
                rejected = true;
                break;
            }
        }
        assert!(rejected);
        assert!(client.poll(Timestamp(1)).is_err());
    });
}
#[test]
fn async_wait_is_idle_after_readiness_and_cancellation_keeps_the_next_event() {
    const CHILD: &str = "WSTOUCHBAR_CLIENT_IDLE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "async_wait_is_idle_after_readiness_and_cancellation_keeps_the_next_event",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("idle transport blocked its executor")
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    futures_lite::future::block_on(async {
        let (mut fake, mut client) = connected().await;
        drain(&mut client, 0);
        let clock = touchbar_session::ManualClock::default();
        let peer = async {
            async_io::Timer::after(Duration::from_millis(1)).await;
            fake.send_message(wire::DaemonMessage::KeyEvent {
                key: WatchKey::Fn,
                phase: KeyPhase::Pressed,
            })
            .unwrap();
        };
        let (event, ()) = futures_lite::future::zip(client.next_event(&clock), peer).await;
        event.unwrap();
        let cpu = || {
            let t = rustix::time::clock_gettime(rustix::time::ClockId::ThreadCPUTime);
            t.tv_sec * 1_000_000_000 + t.tv_nsec
        };
        let before = cpu();
        let timed_out = futures_lite::future::or(
            async {
                async_io::Timer::after(Duration::from_millis(200)).await;
                true
            },
            async {
                client.next_event(&clock).await.unwrap();
                false
            },
        )
        .await;
        assert!(timed_out);
        assert!(cpu() - before < 60_000_000, "idle client is spinning");
        fake.send_message(wire::DaemonMessage::KeyEvent {
            key: WatchKey::Fn,
            phase: KeyPhase::Released,
        })
        .unwrap();
        assert!(matches!(
            client.next_event(&clock).await.unwrap(),
            ClientEvent::Surface(touchbar_session::surface::SurfaceEvent::Input(
                touchbar_core::Input::Key {
                    source: touchbar_core::KeySource::Surface(SurfaceId(20)),
                    code: WatchKey::Fn,
                    phase: KeyPhase::Released
                }
            ))
        ));
    });
}
#[test]
fn unchanged_frame_releases_prepared_slot_without_spending_a_wire_frame_number() {
    futures_lite::future::block_on(async {
        let (mut fake, mut client) = connected().await;
        drain(&mut client, 0);
        fake.set_delays(Duration::ZERO, Duration::ZERO);
        let h = client.handle();
        let first = h.prepare_frame().unwrap().unwrap();
        assert!(
            h.finish_unchanged(first.index).is_err(),
            "first frame must initialize the display"
        );
        h.submit(Generation(7), first.index, &[]).unwrap();
        drain(&mut client, 0);
        fake.next_event().await.unwrap();
        fake.next_event().await.unwrap();
        drain(&mut client, 1);
        let same = h.prepare_frame().unwrap().unwrap();
        h.finish_unchanged(same.index).unwrap();
        drain(&mut client, 2);
        assert_eq!(
            fake.take_received()
                .iter()
                .filter(|m| matches!(m, In::FrameReady { .. }))
                .count(),
            1
        );
        let changed = h.prepare_frame().unwrap().unwrap();
        assert_eq!(same.index, changed.index);
        h.submit(Generation(8), changed.index, &[]).unwrap();
        drain(&mut client, 3);
        assert!(matches!(
            fake.next_event().await.unwrap(),
            Peer::Received(In::FrameReady { frame: 2, .. })
        ));
    });
}
#[test]
fn first_frame_requests_a_full_update_without_damage_messages() {
    futures_lite::future::block_on(async {
        let (mut fake, mut client) = connected().await;
        drain(&mut client, 0);
        let h = client.handle();
        let slot = h.prepare_frame().unwrap().unwrap();
        h.submit(
            Generation(1),
            slot.index,
            &[wire::DamageRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }],
        )
        .unwrap();
        drain(&mut client, 0);
        assert!(
            matches!(
                fake.next_event().await.unwrap(),
                Peer::Received(In::FrameReady { frame: 1, .. })
            ),
            "the initial full frame must not start with FrameDamage"
        );
    });
}
