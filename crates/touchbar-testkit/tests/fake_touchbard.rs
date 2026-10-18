use std::{
    fs::File,
    os::{fd::OwnedFd, unix::fs::FileExt},
    time::{Duration, Instant},
};
use touchbar_model::{KeyPhase, SystemKey, WatchKey};
use touchbar_protocol::{
    self as wire, DaemonMessage as Out, DaemonPacket, DamageRect, Mode, ModeReason,
    SessionMessage as In, SwapchainHeader, XRGB8888,
};
use touchbar_testkit::{FakeTouchbarDisplay, FakeTouchbard, FakeTouchbardEvent as Event};
fn display(rotated: bool) -> FakeTouchbarDisplay {
    let (w, h) = if rotated { (4, 16) } else { (16, 4) };
    FakeTouchbarDisplay {
        header: SwapchainHeader {
            logical_width: 16,
            logical_height: 4,
            physical_width: w,
            physical_height: h,
            format: XRGB8888,
            pitch: w * 4,
            buffer_size: u64::from(w * h * 4),
            buffer_count: 3,
            frame_damage: true,
        },
        width_mm100: 24000,
        height_mm100: 600,
    }
}
async fn connected(rotated: bool) -> (FakeTouchbard, OwnedFd, Vec<File>) {
    connected_info(display(rotated)).await
}
async fn connected_info(info: FakeTouchbarDisplay) -> (FakeTouchbard, OwnedFd, Vec<File>) {
    let (mut fake, fd) = FakeTouchbard::pair(info).unwrap();
    wire::send_session(&fd, In::Hello { version: 2 }).unwrap();
    assert!(matches!(
        fake.next_event().await.unwrap(),
        Event::Received(In::Hello { version: 2 })
    ));
    assert!(matches!(
        wire::recv_daemon(&fd).unwrap(),
        DaemonPacket::Message(Out::ModeChanged {
            mode: Mode::Native,
            ..
        })
    ));
    fake.set_mode(Mode::Display, ModeReason::Owner).unwrap();
    assert!(matches!(
        wire::recv_daemon(&fd).unwrap(),
        DaemonPacket::Message(Out::Welcome {
            width: 16,
            height: 4,
            width_mm100: 24000,
            height_mm100: 600,
            ..
        })
    ));
    let DaemonPacket::Swapchain { header, buffers } = wire::recv_daemon(&fd).unwrap() else {
        panic!("swapchain")
    };
    assert_eq!(header, info.header);
    assert!(matches!(
        wire::recv_daemon(&fd).unwrap(),
        DaemonPacket::Message(Out::ModeChanged {
            mode: Mode::Display,
            ..
        })
    ));
    (fake, fd, buffers.into_iter().map(File::from).collect())
}
#[test]
fn real_memfd_swapchain_records_messages_and_preserves_pixels_in_both_orientations() {
    futures_lite::future::block_on(async {
        for rotated in [false, true] {
            let (mut fake, fd, buffers) = connected(rotated).await;
            let pixels = (0u8..=255).collect::<Vec<_>>();
            buffers[0].write_all_at(&pixels, 0).unwrap();
            assert_eq!(fake.pixels(0).unwrap(), pixels);
            wire::send_session(
                &fd,
                In::FrameDamage(DamageRect {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2,
                }),
            )
            .unwrap();
            assert!(matches!(
                fake.next_event().await.unwrap(),
                Event::Received(In::FrameDamage(_))
            ));
            wire::send_session(&fd, In::FrameReady { index: 0, frame: 1 }).unwrap();
            assert!(matches!(
                fake.next_event().await.unwrap(),
                Event::Received(In::FrameReady { .. })
            ));
            assert!(matches!(
                fake.next_event().await.unwrap(),
                Event::Presented { index: 0, frame: 1 }
            ));
            assert!(matches!(
                wire::recv_daemon(&fd).unwrap(),
                DaemonPacket::Message(Out::FramePresented {
                    index: 0,
                    frame: 1,
                    ..
                })
            ));
            wire::send_session(
                &fd,
                In::WatchKey {
                    key: WatchKey::Fn,
                    enabled: true,
                },
            )
            .unwrap();
            let _ = fake.next_event().await.unwrap();
            let _ = wire::recv_daemon(&fd).unwrap();
            wire::send_session(
                &fd,
                In::Key {
                    key: SystemKey::F5,
                    phase: KeyPhase::Pressed,
                },
            )
            .unwrap();
            let _ = fake.next_event().await.unwrap();
            wire::send_session(&fd, In::Backlight { limit: 1234 }).unwrap();
            let _ = fake.next_event().await.unwrap();
            let recorded = fake.take_received();
            assert!(recorded.contains(&In::Key {
                key: SystemKey::F5,
                phase: KeyPhase::Pressed
            }));
            assert!(recorded.contains(&In::Backlight { limit: 1234 }));
        }
    });
}
#[test]
fn rejects_second_unconfirmed_frame_and_reuse_of_scanout() {
    futures_lite::future::block_on(async {
        let (mut fake, fd, _) = connected(false).await;
        fake.set_delays(Duration::from_secs(1), Duration::ZERO);
        wire::send_session(&fd, In::FrameReady { index: 0, frame: 1 }).unwrap();
        fake.next_event().await.unwrap();
        wire::send_session(&fd, In::FrameReady { index: 1, frame: 2 }).unwrap();
        assert!(
            fake.next_event()
                .await
                .unwrap_err()
                .to_string()
                .contains("unconfirmed")
        );
        let (mut fake, fd, _) = connected(false).await;
        wire::send_session(&fd, In::FrameReady { index: 0, frame: 1 }).unwrap();
        fake.next_event().await.unwrap();
        fake.next_event().await.unwrap();
        let _ = wire::recv_daemon(&fd).unwrap();
        wire::send_session(&fd, In::FrameReady { index: 0, frame: 2 }).unwrap();
        assert!(
            fake.next_event()
                .await
                .unwrap_err()
                .to_string()
                .contains("held")
        );
    });
}
#[test]
fn rejects_writes_to_a_held_memfd_at_the_next_protocol_boundary() {
    futures_lite::future::block_on(async {
        let (mut fake, fd, buffers) = connected(false).await;
        fake.set_delays(Duration::from_secs(1), Duration::ZERO);
        wire::send_session(&fd, In::FrameReady { index: 0, frame: 1 }).unwrap();
        fake.next_event().await.unwrap();
        buffers[0].write_all_at(&[9, 8, 7, 0], 0).unwrap();
        wire::send_session(&fd, In::Backlight { limit: 123 }).unwrap();
        assert!(
            fake.next_event()
                .await
                .unwrap_err()
                .to_string()
                .contains("held buffer")
        );
    });
}
#[test]
fn rejects_damage_outside_physical_extent_and_native_key_press() {
    futures_lite::future::block_on(async {
        let (mut fake, fd, _) = connected(true).await;
        wire::send_session(
            &fd,
            In::FrameDamage(DamageRect {
                x: 4,
                y: 0,
                width: 1,
                height: 1,
            }),
        )
        .unwrap();
        assert!(fake.next_event().await.is_err());
        fake.set_mode(Mode::Native, ModeReason::Recovery).unwrap();
        wire::send_session(
            &fd,
            In::Key {
                key: SystemKey::VolumeUp,
                phase: KeyPhase::Pressed,
            },
        )
        .unwrap();
        assert!(
            fake.next_event()
                .await
                .unwrap_err()
                .to_string()
                .contains("native")
        );
    });
}
#[test]
fn presentation_and_release_use_independent_scenario_delays() {
    futures_lite::future::block_on(async {
        let (mut fake, fd, _) = connected(false).await;
        fake.set_delays(Duration::from_millis(20), Duration::from_millis(30));
        let start = Instant::now();
        wire::send_session(&fd, In::FrameReady { index: 0, frame: 1 }).unwrap();
        fake.next_event().await.unwrap();
        fake.next_event().await.unwrap();
        assert!(start.elapsed() >= Duration::from_millis(20));
        let _ = wire::recv_daemon(&fd).unwrap();
        let second_started = Instant::now();
        wire::send_session(&fd, In::FrameReady { index: 1, frame: 2 }).unwrap();
        fake.next_event().await.unwrap();
        assert!(matches!(
            fake.next_event().await.unwrap(),
            Event::Presented { index: 1, frame: 2 }
        ));
        let _ = wire::recv_daemon(&fd).unwrap();
        assert!(matches!(
            fake.next_event().await.unwrap(),
            Event::Released { index: 0, frame: 1 }
        ));
        assert!(second_started.elapsed() >= Duration::from_millis(50));
        assert!(matches!(
            wire::recv_daemon(&fd).unwrap(),
            DaemonPacket::Message(Out::BufferReleased { index: 0, frame: 1 })
        ));
        wire::send_session(&fd, In::FrameReady { index: 0, frame: 3 }).unwrap();
        assert!(matches!(
            fake.next_event().await.unwrap(),
            Event::Received(In::FrameReady { frame: 3, .. })
        ));
    });
}

async fn present(fake: &mut FakeTouchbard, fd: &OwnedFd, index: u32, frame: u64) {
    wire::send_session(fd, In::FrameReady { index, frame }).unwrap();
    assert!(matches!(
        fake.next_event().await.unwrap(),
        Event::Received(In::FrameReady { .. })
    ));
    assert!(matches!(
        fake.next_event().await.unwrap(),
        Event::Presented { .. }
    ));
    assert!(matches!(
        wire::recv_daemon(fd).unwrap(),
        DaemonPacket::Message(Out::FramePresented { .. })
    ));
}
#[test]
fn release_schedule_can_free_buffers_in_a_different_order() {
    futures_lite::future::block_on(async {
        let (mut fake, fd, _) = connected(false).await;
        fake.set_delays(Duration::ZERO, Duration::from_secs(10));
        present(&mut fake, &fd, 0, 1).await;
        present(&mut fake, &fd, 1, 2).await;
        fake.set_delays(Duration::ZERO, Duration::ZERO);
        present(&mut fake, &fd, 2, 3).await;
        let event = futures_lite::future::or(fake.next_event(), async {
            async_io::Timer::after(Duration::from_millis(100)).await;
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "ready release was hidden behind older delayed release",
            ))
        })
        .await
        .unwrap();
        assert_eq!(event, Event::Released { index: 1, frame: 2 });
    });
}

#[test]
fn physical_key_snapshot_is_sent_only_after_watch_is_enabled() {
    futures_lite::future::block_on(async {
        let (mut fake, fd) = FakeTouchbard::pair(display(false)).unwrap();
        fake.set_physical_key(WatchKey::Fn, KeyPhase::Pressed)
            .unwrap();
        wire::send_session(&fd, In::Hello { version: 2 }).unwrap();
        fake.next_event().await.unwrap();
        assert!(matches!(
            wire::recv_daemon(&fd).unwrap(),
            DaemonPacket::Message(Out::ModeChanged {
                mode: Mode::Native,
                ..
            })
        ));
        wire::send_session(
            &fd,
            In::WatchKey {
                key: WatchKey::Fn,
                enabled: true,
            },
        )
        .unwrap();
        fake.next_event().await.unwrap();
        assert!(matches!(
            wire::recv_daemon(&fd).unwrap(),
            DaemonPacket::Message(Out::KeyEvent {
                key: WatchKey::Fn,
                phase: KeyPhase::Pressed
            })
        ));
        fake.set_physical_key(WatchKey::Esc, KeyPhase::Pressed)
            .unwrap();
        fake.set_physical_key(WatchKey::Fn, KeyPhase::Released)
            .unwrap();
        assert!(matches!(
            wire::recv_daemon(&fd).unwrap(),
            DaemonPacket::Message(Out::KeyEvent {
                key: WatchKey::Fn,
                phase: KeyPhase::Released
            })
        ));
    });
}

fn client_connect(path: &std::path::Path) -> OwnedFd {
    use rustix::net::*;
    let fd = socket(AddressFamily::UNIX, SocketType::SEQPACKET, None).unwrap();
    rustix::net::connect(&fd, &SocketAddrUnix::new(path).unwrap()).unwrap();
    fd
}
#[test]
fn listener_reconnect_allocates_fresh_memfds_and_cleans_its_socket() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.sock");
        let mut fake = FakeTouchbard::bind(&path, display(false)).unwrap();
        fake.set_mode(Mode::Display, ModeReason::Owner).unwrap();
        let mut old_buffer = None;
        for _ in 0..2 {
            let fd = client_connect(&path);
            assert_eq!(fake.next_event().await.unwrap(), Event::Connected);
            wire::send_session(&fd, In::Hello { version: 2 }).unwrap();
            fake.next_event().await.unwrap();
            let _ = wire::recv_daemon(&fd).unwrap();
            let _ = wire::recv_daemon(&fd).unwrap();
            let DaemonPacket::Swapchain { buffers, .. } = wire::recv_daemon(&fd).unwrap() else {
                panic!("swapchain")
            };
            let current = File::from(buffers.into_iter().next().unwrap());
            assert!(
                current.set_len(0).is_err(),
                "client could truncate sealed memfd"
            );
            if let Some(old) = old_buffer.take() {
                let old: File = old;
                old.write_all_at(&[99; 4], 0).unwrap();
                assert_eq!(fake.pixels(0).unwrap()[..4], [0; 4]);
            }
            old_buffer = Some(current);
            let _ = wire::recv_daemon(&fd).unwrap();
            drop(fd);
            assert_eq!(fake.next_event().await.unwrap(), Event::Disconnected);
        }
        drop(fake);
        assert!(!path.exists());
    });
}
#[test]
fn scenario_sends_touch_key_and_terminal_revocation() {
    futures_lite::future::block_on(async {
        let (mut fake, fd, _) = connected(false).await;
        let messages = [
            Out::Touch {
                phase: touchbar_model::TouchPhase::Down,
                contact: std::num::NonZeroU32::new(1).unwrap(),
                x: 2,
                y: 1,
            },
            Out::KeyEvent {
                key: WatchKey::Fn,
                phase: KeyPhase::Pressed,
            },
            Out::Revoked(wire::RevokedReason::SessionInactive),
        ];
        for message in messages {
            fake.send_message(message).unwrap();
            assert!(
                matches!(wire::recv_daemon(&fd).unwrap(),DaemonPacket::Message(v) if v==message)
            );
        }
        assert!(wire::recv_daemon(&fd).is_err());
    });
}
#[test]
fn bad_hello_index_frame_order_and_damage_count_are_rejected() {
    futures_lite::future::block_on(async {
        let (mut fake, fd) = FakeTouchbard::pair(display(false)).unwrap();
        wire::send_session(&fd, In::Hello { version: 1 }).unwrap();
        assert!(fake.next_event().await.is_err());
        let mut info = display(false);
        info.header.buffer_count = 2;
        let (mut fake, fd, _) = connected_info(info).await;
        wire::send_session(&fd, In::FrameReady { index: 2, frame: 1 }).unwrap();
        assert!(fake.next_event().await.is_err());
        let (mut fake, fd, _) = connected(false).await;
        present(&mut fake, &fd, 0, 1).await;
        wire::send_session(&fd, In::FrameReady { index: 1, frame: 1 }).unwrap();
        assert!(fake.next_event().await.is_err());
        let (mut fake, fd, _) = connected(false).await;
        for i in 0..9 {
            wire::send_session(
                &fd,
                In::FrameDamage(DamageRect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                }),
            )
            .unwrap();
            assert_eq!(fake.next_event().await.is_ok(), i < 8);
        }
    });
}

#[test]
fn damage_requires_the_advertised_header_capability() {
    futures_lite::future::block_on(async {
        let mut info = display(false);
        info.header.frame_damage = false;
        let (mut fake, fd, _) = connected_info(info).await;
        wire::send_session(
            &fd,
            In::FrameDamage(DamageRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }),
        )
        .unwrap();
        assert!(fake.next_event().await.is_err());
    });
}

#[test]
fn delay_changes_apply_to_future_frames_and_preserve_pending_commit_measurement() {
    futures_lite::future::block_on(async {
        let (mut fake, fd, _) = connected(false).await;
        fake.set_delays(Duration::from_millis(20), Duration::ZERO);
        wire::send_session(&fd, In::FrameReady { index: 0, frame: 1 }).unwrap();
        fake.next_event().await.unwrap();
        fake.set_delays(Duration::ZERO, Duration::ZERO);
        fake.next_event().await.unwrap();
        assert!(matches!(
            wire::recv_daemon(&fd).unwrap(),
            DaemonPacket::Message(Out::FramePresented {
                commit_us: 20_000,
                ..
            })
        ));
    });
}

#[test]
fn pending_replacement_keeps_its_original_release_delay() {
    futures_lite::future::block_on(async {
        let (mut fake, fd, _) = connected(false).await;
        present(&mut fake, &fd, 0, 1).await;
        fake.set_delays(Duration::from_millis(20), Duration::from_millis(30));
        let started = Instant::now();
        wire::send_session(&fd, In::FrameReady { index: 1, frame: 2 }).unwrap();
        fake.next_event().await.unwrap();
        fake.set_delays(Duration::ZERO, Duration::ZERO);
        fake.next_event().await.unwrap();
        let _ = wire::recv_daemon(&fd).unwrap();
        assert!(matches!(
            fake.next_event().await.unwrap(),
            Event::Released { index: 0, frame: 1 }
        ));
        assert!(started.elapsed() >= Duration::from_millis(50));
    });
}
