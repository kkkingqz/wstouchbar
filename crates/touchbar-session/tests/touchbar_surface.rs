use std::{rc::Rc, sync::Arc};
use touchbar_core::Input;
use touchbar_model::*;
use touchbar_protocol::{self as wire, Mode, ModeReason, SessionMessage as In};
use touchbar_session::{
    surface::*,
    touchbard::{Client, ClientEvent, TouchbarSurface},
    *,
};
use touchbar_testkit::{FakeTouchbarDisplay, FakeTouchbard, FakeTouchbardEvent as Peer};
// The protocol receives real primitive packets; only GL pixel production is replaced.
struct Primitives(TouchbarSurface);
impl Surface for Primitives {
    fn info(&self) -> SurfaceInfo {
        self.0.info()
    }
    fn present(&mut self, frame: Arc<PresentedFrame>) -> Result<Presentation, String> {
        if self.0.prepare_shared_target()?.is_none() {
            return Ok(Presentation::Pending);
        }
        let mut f = (*frame).clone();
        f.target = Some(touchbar_gl::TargetFrame {
            damage: vec![],
            copied: vec![],
        });
        self.0.present(Arc::new(f))
    }
    fn watch_keys(&mut self, k: &[WatchKey]) -> Result<(), String> {
        self.0.watch_keys(k)
    }
    fn send_key(&mut self, k: SystemKey, p: KeyPhase) -> Result<(), String> {
        self.0.send_key(k, p)
    }
    fn backlight(&mut self, l: f32) -> Result<(), String> {
        self.0.backlight(l)
    }
    fn close(&mut self) {
        self.0.close()
    }
}
fn display() -> FakeTouchbarDisplay {
    FakeTouchbarDisplay {
        header: wire::SwapchainHeader {
            logical_width: 400,
            logical_height: 60,
            physical_width: 400,
            physical_height: 60,
            format: wire::XRGB8888,
            pitch: 1600,
            buffer_size: 96000,
            buffer_count: 3,
            frame_damage: true,
        },
        width_mm100: 20000,
        height_mm100: 3000,
    }
}
fn session(path: &std::path::Path, clock: &ManualClock) -> Session {
    let desktop = Rc::new(touchbar_desktop::NullBackend::default());
    let (effects, _) = touchbar_effects::Engine::fake(desktop.clone());
    Session::new(
        SessionOptions::new(path.into(), touchbar_loader::UserConfig::default()),
        clock.clone(),
        Box::<touchbar_logic_runtime::MemoryStore>::default(),
        desktop,
        effects,
    )
    .unwrap()
}
fn deliver(client: &mut Client, session: &mut Session, clock: &ManualClock) {
    for _ in 0..48 {
        match client.poll(clock.now()).unwrap() {
            Some(ClientEvent::Welcome(_)) => session
                .add_surface(Box::new(Primitives(
                    TouchbarSurface::new(client.handle()).unwrap(),
                )))
                .unwrap(),
            Some(ClientEvent::Surface(e)) => session.handle_event(Event::Surface(e)).unwrap(),
            None => {}
        }
    }
    session.pump().unwrap();
}
#[test]
fn frame_done_after_presented_and_native_stops_frames_until_fresh_swapchain() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let clock = ManualClock::default();
        let mut session = session(dir.path(), &clock);
        let (mut fake, fd) = FakeTouchbard::pair(display()).unwrap();
        let mut client = Client::from_fd(fd, SurfaceId(20)).unwrap();
        deliver(&mut client, &mut session, &clock);
        fake.next_event().await.unwrap();
        fake.set_mode(Mode::Display, ModeReason::Owner).unwrap();
        deliver(&mut client, &mut session, &clock);
        deliver(&mut client, &mut session, &clock);
        let mut accepted = false;
        for _ in 0..16 {
            if matches!(
                fake.next_event().await.unwrap(),
                Peer::Received(In::FrameReady { frame: 1, .. })
            ) {
                accepted = true;
                break;
            }
        }
        assert!(accepted);
        assert!(
            !session
                .dump()
                .inputs
                .iter()
                .any(|e| matches!(e.input, Input::FrameDone { .. }))
        );
        fake.next_event().await.unwrap();
        deliver(&mut client, &mut session, &clock);
        assert!(session.dump().inputs.iter().any(|e| matches!(
            e.input,
            Input::FrameDone {
                surface: SurfaceId(20),
                ..
            }
        )));
        // A second frame is accepted but never acknowledged before native.
        session
            .handle_event(Event::Input(Input::Fact {
                name: "appearance.scheme".into(),
                value: Some(FactValue::Text("light".into())),
            }))
            .unwrap();
        session.pump().unwrap();
        deliver(&mut client, &mut session, &clock);
        assert!(matches!(
            fake.next_event().await.unwrap(),
            Peer::Received(In::FrameReady { frame: 2, .. })
        ));
        fake.set_mode(Mode::Native, ModeReason::Recovery).unwrap();
        deliver(&mut client, &mut session, &clock);
        assert!(session.frame(SurfaceId(20)).is_none());
        fake.set_mode(Mode::Display, ModeReason::Owner).unwrap();
        deliver(&mut client, &mut session, &clock);
        deliver(&mut client, &mut session, &clock);
        assert!(
            session.frame(SurfaceId(20)).is_some(),
            "native must cancel the previous busy pacer"
        );
        session.dump().replay().unwrap();
    });
}
#[test]
#[ignore = "requires EGL; exercises the actual shared Session renderer, no Wayland or devices"]
fn env_gpu_stale_buffers_match_current_frame_in_both_orientations() {
    futures_lite::future::block_on(async {
        for rotated in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let clock = ManualClock::default();
            let mut session = session(dir.path(), &clock);
            let mut d = display();
            if rotated {
                d.header.physical_width = 60;
                d.header.physical_height = 400;
                d.header.pitch = 240;
            }
            let (mut fake, fd) = FakeTouchbard::pair(d).unwrap();
            fake.set_delays(
                std::time::Duration::ZERO,
                std::time::Duration::from_millis(3),
            );
            let mut client = Client::from_fd(fd, SurfaceId(20)).unwrap();
            client.poll(clock.now()).unwrap();
            fake.next_event().await.unwrap();
            fake.set_mode(Mode::Display, ModeReason::Owner).unwrap();
            let mut oracle = touchbar_gl::Renderer::new(touchbar_gl::Backend::Surfaceless).unwrap();
            let mut submitted_indices = Vec::new();
            for n in 0..6 {
                // Keep buffer 0 held while buffer 1 is released first after frame 3.
                fake.set_delays(
                    std::time::Duration::ZERO,
                    if n == 1 {
                        std::time::Duration::from_millis(500)
                    } else {
                        std::time::Duration::ZERO
                    },
                );
                if n > 0 {
                    clock.advance(17);
                    session
                        .handle_event(Event::Input(Input::Fact {
                            name: "appearance.scheme".into(),
                            value: Some(FactValue::Text(
                                if n % 2 == 0 { "dark" } else { "light" }.into(),
                            )),
                        }))
                        .unwrap();
                    session.pump().unwrap();
                }
                let mut submitted = None;
                for _ in 0..100 {
                    for _ in 0..16 {
                        match client.poll(clock.now()).unwrap() {
                            Some(ClientEvent::Welcome(_)) => session
                                .add_surface(Box::new(
                                    TouchbarSurface::new(client.handle()).unwrap(),
                                ))
                                .unwrap(),
                            Some(ClientEvent::Surface(e)) => {
                                session.handle_event(Event::Surface(e)).unwrap()
                            }
                            None => {}
                        }
                    }
                    session.pump().unwrap();
                    if let Some(event) = futures_lite::future::poll_once(fake.next_event()).await
                        && let Peer::Received(In::FrameReady { index, frame }) = event.unwrap()
                    {
                        submitted = Some((index, frame));
                        break;
                    }
                    async_io::Timer::after(std::time::Duration::from_millis(1)).await;
                }
                let (index, frame) = submitted.expect("frame submitted after buffer release");
                submitted_indices.push(index);
                assert_eq!(frame, n + 1);
                assert_eq!(client.status().path.as_deref(), Some("copy"));
                let packet = session.frame(SurfaceId(20)).unwrap();
                let reference = oracle
                    .render(
                        SurfaceId(2),
                        (400, 60),
                        touchbar_gl::Orientation::Landscape,
                        &packet.display_list,
                        true,
                    )
                    .unwrap()
                    .unwrap();
                let pixels = fake.pixels(index).unwrap();
                for y in 0..60 {
                    for x in 0..400 {
                        let src = (y * 400 + x) * 4;
                        let dst = if rotated {
                            x * 240 + y * 4
                        } else {
                            y * 1600 + x * 4
                        };
                        assert_eq!(
                            &pixels[dst..dst + 4],
                            &[
                                reference.pixels[src + 2],
                                reference.pixels[src + 1],
                                reference.pixels[src],
                                255
                            ],
                            "frame {frame} pixel {x},{y}"
                        );
                    }
                }
                loop {
                    if matches!(fake.next_event().await.unwrap(),Peer::Presented{frame:f,..}if f==frame)
                    {
                        break;
                    }
                }
                for _ in 0..8 {
                    if let Some(ClientEvent::Surface(e)) = client.poll(clock.now()).unwrap() {
                        session.handle_event(Event::Surface(e)).unwrap();
                    }
                }
                session.pump().unwrap();
            }
            assert_eq!(
                &submitted_indices[..4],
                &[0, 1, 2, 1],
                "release order must control buffer reuse"
            );
            session.dump().replay().unwrap();
            session.shutdown().unwrap();
        }
    });
}
mod support;
async fn wire_settle(f: &mut support::Fixture, client: &mut Client, fake: &mut FakeTouchbard) {
    for _ in 0..64 {
        deliver(client, &mut f.session, &f.clock);
        f.settle();
        for _ in 0..32 {
            if futures_lite::future::poll_once(fake.next_event())
                .await
                .transpose()
                .unwrap()
                .is_none()
            {
                break;
            }
        }
    }
}
async fn controls_wire() -> (support::Fixture, Client, FakeTouchbard) {
    let mut f = support::Fixture::new(2008, 60, false);
    f.send(Input::SurfaceRemoved(SurfaceId(1)));
    let mut d = display();
    d.header.logical_width = 2008;
    d.header.physical_width = 2008;
    d.header.pitch = 8032;
    d.header.buffer_size = 481920;
    let (mut fake, fd) = FakeTouchbard::pair(d).unwrap();
    fake.set_delays(std::time::Duration::ZERO, std::time::Duration::ZERO);
    let mut client = Client::from_fd(fd, SurfaceId(20)).unwrap();
    client.poll(f.clock.now()).unwrap();
    fake.next_event().await.unwrap();
    fake.set_mode(Mode::Display, ModeReason::Owner).unwrap();
    wire_settle(&mut f, &mut client, &mut fake).await;
    (f, client, fake)
}
fn rect(f: &support::Fixture, id: &str) -> Rect {
    f.session
        .frame(SurfaceId(20))
        .unwrap()
        .layout
        .hit_map
        .iter()
        .find(|h| h.widget.as_str() == id)
        .unwrap()
        .rect
}
#[allow(clippy::cast_possible_truncation)]
fn touch(fake: &mut FakeTouchbard, phase: TouchPhase, id: u32, x: f32, y: f32) {
    fake.send_message(wire::DaemonMessage::Touch {
        phase,
        contact: std::num::NonZeroU32::new(id).unwrap(),
        x: x.round() as i32,
        y: y.round() as i32,
    })
    .unwrap();
}
#[test]
fn fn_from_touchbard_shows_fkeys_and_watch_keys_follow_bindings() {
    futures_lite::future::block_on(async {
        let (mut f, mut client, mut fake) = controls_wire().await;
        assert!(fake.take_received().contains(&In::WatchKey {
            key: WatchKey::Fn,
            enabled: true
        }));
        fake.set_physical_key(WatchKey::Fn, KeyPhase::Pressed)
            .unwrap();
        wire_settle(&mut f, &mut client, &mut fake).await;
        let frame = f.session.frame(SurfaceId(20)).unwrap();
        assert_eq!(frame.layout.hit_map.len(), 12);
        let r = frame.layout.hit_map[4].rect;
        touch(
            &mut fake,
            TouchPhase::Down,
            1,
            r.x + r.width * 0.5,
            r.y + r.height * 0.5,
        );
        wire_settle(&mut f, &mut client, &mut fake).await;
        touch(
            &mut fake,
            TouchPhase::Up,
            1,
            r.x + r.width * 0.5,
            r.y + r.height * 0.5,
        );
        wire_settle(&mut f, &mut client, &mut fake).await;
        let messages = fake.take_received();
        assert!(messages.contains(&In::Key {
            key: SystemKey::F5,
            phase: KeyPhase::Pressed
        }));
        assert!(messages.contains(&In::Key {
            key: SystemKey::F5,
            phase: KeyPhase::Released
        }));
        std::fs::write(f.file("system/bindings.toml"),"[[binding]]\nid='fn-layer'\ntrigger={key='left-shift',gesture='while-held'}\naction={layer='system/fkeys'}\n").unwrap();
        assert!(f.reload().accepted);
        wire_settle(&mut f, &mut client, &mut fake).await;
        let messages = fake.take_received();
        assert!(messages.contains(&In::WatchKey {
            key: WatchKey::Fn,
            enabled: false
        }));
        assert!(messages.contains(&In::WatchKey {
            key: WatchKey::LeftShift,
            enabled: true
        }));
        f.checked();
    });
}
#[test]
fn touch_reaches_core_and_volume_effect_then_revocation_cancels_capture() {
    futures_lite::future::block_on(async {
        let (mut f, mut client, mut fake) = controls_wire().await;
        let r = rect(&f, "volume-panel");
        touch(
            &mut fake,
            TouchPhase::Down,
            1,
            r.x + r.width * 0.5,
            r.y + r.height * 0.5,
        );
        wire_settle(&mut f, &mut client, &mut fake).await;
        f.advance(420);
        wire_settle(&mut f, &mut client, &mut fake).await;
        let r = rect(&f, "volume-panel-level");
        touch(
            &mut fake,
            TouchPhase::Down,
            2,
            r.x + r.width * 0.8,
            r.y + r.height * 0.5,
        );
        wire_settle(&mut f, &mut client, &mut fake).await;
        assert!(
            f.effects
                .started()
                .iter()
                .any(|(_, effect)| matches!(effect,Effect::AudioSetVolume{level,..}if *level>0.5))
        );
        assert!(!f.session.snapshot().captures.is_empty());
        fake.send_message(wire::DaemonMessage::Revoked(
            wire::RevokedReason::SessionInactive,
        ))
        .unwrap();
        assert!(client.poll(f.clock.now()).is_err());
        f.send(Input::Revoked {
            surface: SurfaceId(20),
        });
        f.send(Input::SurfaceRemoved(SurfaceId(20)));
        let snapshot = f.session.snapshot();
        assert!(snapshot.surfaces.is_empty());
        assert!(snapshot.captures.is_empty());
        assert!(snapshot.instances.is_empty());
        f.checked();
    });
}
async fn gpu_step(client: &mut Client, session: &mut Session, clock: &ManualClock) {
    for _ in 0..16 {
        match client.poll(clock.now()).unwrap() {
            Some(ClientEvent::Welcome(_)) => session
                .add_surface(Box::new(TouchbarSurface::new(client.handle()).unwrap()))
                .unwrap(),
            Some(ClientEvent::Surface(e)) => session.handle_event(Event::Surface(e)).unwrap(),
            None => {}
        }
    }
    session.pump().unwrap();
}
async fn gpu_accept(
    client: &mut Client,
    session: &mut Session,
    clock: &ManualClock,
    fake: &mut FakeTouchbard,
) -> u64 {
    for _ in 0..100 {
        gpu_step(client, session, clock).await;
        if let Some(event) = futures_lite::future::poll_once(fake.next_event()).await
            && let Peer::Received(In::FrameReady { frame, .. }) = event.unwrap()
        {
            return frame;
        }
        async_io::Timer::after(std::time::Duration::from_millis(1)).await;
    }
    panic!("GPU frame was not submitted")
}
#[test]
#[ignore = "requires EGL, no real devices"]
fn env_gpu_unchanged_render_completes_locally_and_next_changed_frame_is_submitted() {
    futures_lite::future::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let clock = ManualClock::default();
        let mut session = session(dir.path(), &clock);
        let (mut fake, fd) = FakeTouchbard::pair(display()).unwrap();
        fake.set_delays(std::time::Duration::ZERO, std::time::Duration::ZERO);
        let mut client = Client::from_fd(fd, SurfaceId(20)).unwrap();
        client.poll(clock.now()).unwrap();
        fake.next_event().await.unwrap();
        fake.set_mode(Mode::Display, ModeReason::Owner).unwrap();
        assert_eq!(
            gpu_accept(&mut client, &mut session, &clock, &mut fake).await,
            1
        );
        fake.next_event().await.unwrap();
        gpu_step(&mut client, &mut session, &clock).await;
        let count = |s: &Session| {
            s.dump()
                .inputs
                .iter()
                .filter(|e| {
                    matches!(
                        e.input,
                        Input::FrameDone {
                            surface: SurfaceId(20),
                            ..
                        }
                    )
                })
                .count()
        };
        let before = count(&session);
        for theme in ["light", "dark"] {
            session
                .handle_event(Event::Input(Input::Fact {
                    name: "appearance.scheme".into(),
                    value: Some(FactValue::Text(theme.into())),
                }))
                .unwrap();
        }
        session.pump().unwrap();
        for _ in 0..4 {
            gpu_step(&mut client, &mut session, &clock).await;
            if let Some(event) = futures_lite::future::poll_once(fake.next_event()).await {
                assert!(
                    !matches!(event.unwrap(), Peer::Received(In::FrameReady { .. })),
                    "unchanged image reached the daemon"
                );
            }
        }
        assert_eq!(count(&session), before + 1);
        assert_eq!(client.status().commit_us.samples, 1);
        session
            .handle_event(Event::Input(Input::Fact {
                name: "appearance.scheme".into(),
                value: Some(FactValue::Text("light".into())),
            }))
            .unwrap();
        assert_eq!(
            gpu_accept(&mut client, &mut session, &clock, &mut fake).await,
            2
        );
        session.dump().replay().unwrap();
    });
}
