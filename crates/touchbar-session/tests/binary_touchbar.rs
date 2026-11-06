//! The production binary, a real seqpacket socket, standard plugins and portrait memfds.
use futures_lite::future::or;
use std::{
    collections::BTreeMap,
    fs,
    io::BufReader,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use touchbar_model::{KeyPhase, SystemKey, TouchPhase, WatchKey};
use touchbar_protocol::{
    self as wire, DaemonMessage as Out, DamageRect, Mode, ModeReason, SessionMessage as In,
};
use touchbar_testkit::{FakeTouchbarDisplay, FakeTouchbard, FakeTouchbardEvent as Peer};
const LIMIT: Duration = Duration::from_secs(30);
const WIDTH: u32 = 2008;
const HEIGHT: u32 = 60;
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Image {
    pixels: Vec<u8>,
    surface: u64,
    generation: u64,
}
struct Binary {
    dir: tempfile::TempDir,
    child: ChildGuard,
    fake: FakeTouchbard,
    pending: BTreeMap<u64, (Vec<u8>, Vec<DamageRect>)>,
    damage: Vec<DamageRect>,
    last: Option<(Vec<u8>, Vec<DamageRect>)>,
    check_damage: bool,
    changes: Vec<DamageRect>,
}
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
impl Binary {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for p in [
            "runtime",
            "home",
            "config/touchbar",
            "state",
            "cache",
            "data",
            "fonts",
        ] {
            fs::create_dir_all(dir.path().join(p)).unwrap();
        }
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            dir.path().join("runtime"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        fs::copy(
            root().join("assets/fonts/test/InterVariable.ttf"),
            dir.path().join("fonts/Inter.ttf"),
        )
        .unwrap();
        fs::write(dir.path().join("fonts.conf"),format!("<?xml version=\"1.0\"?><!DOCTYPE fontconfig SYSTEM \"fonts.dtd\"><fontconfig><dir>{}</dir><alias><family>sans-serif</family><prefer><family>Inter Variable</family></prefer></alias></fontconfig>",dir.path().join("fonts").display())).unwrap();
        xtask::build_plugins(&root(), &dir.path().join("plugins")).unwrap();
        let mut fake = FakeTouchbard::bind(
            &dir.path().join("daemon.sock"),
            FakeTouchbarDisplay {
                header: wire::SwapchainHeader {
                    logical_width: WIDTH,
                    logical_height: HEIGHT,
                    physical_width: HEIGHT,
                    physical_height: WIDTH,
                    format: wire::XRGB8888,
                    pitch: 256,
                    buffer_size: 256 * u64::from(WIDTH),
                    buffer_count: 3,
                    frame_damage: true,
                },
                width_mm100: 32387,
                height_mm100: 968,
            },
        )
        .unwrap();
        fake.set_mode(Mode::Display, ModeReason::Owner).unwrap();
        let log = fs::File::create(dir.path().join("session.log")).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_touchbar-session"))
            .args(["--desktop=fake", "--effects=fake", "--plugins"])
            .arg(dir.path().join("plugins"))
            .arg("--touchbard-socket")
            .arg(dir.path().join("daemon.sock"))
            .env("HOME", dir.path().join("home"))
            .env("XDG_RUNTIME_DIR", dir.path().join("runtime"))
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_STATE_HOME", dir.path().join("state"))
            .env("XDG_CACHE_HOME", dir.path().join("cache"))
            .env("XDG_DATA_HOME", dir.path().join("data"))
            .env("FONTCONFIG_FILE", dir.path().join("fonts.conf"))
            .env("LANG", "en_US.UTF-8")
            .env("LC_ALL", "en_US.UTF-8")
            .env("TOUCHBAR_LOG", "warn")
            .env_remove("JOURNAL_STREAM")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap();
        Self {
            dir,
            child: ChildGuard(child),
            fake,
            pending: BTreeMap::new(),
            damage: Vec::new(),
            last: None,
            check_damage: false,
            changes: Vec::new(),
        }
    }
    fn log(&self) -> String {
        fs::read_to_string(self.dir.path().join("session.log")).unwrap()
    }
    async fn control(&self, c: touchbar_control::Command) -> touchbar_control::Reply {
        let path = touchbar_control::socket_path(&self.dir.path().join("runtime"), false);
        or(
            async {
                touchbar_control::call(&path, c)
                    .await
                    .unwrap_or_else(|e| panic!("{e}: {}", self.log()))
            },
            async {
                async_io::Timer::after(Duration::from_secs(5)).await;
                panic!("control timeout: {}", self.log())
            },
        )
        .await
    }
    async fn status(&self) -> serde_json::Value {
        match self.control(touchbar_control::Command::Status).await {
            touchbar_control::Reply::Status(v) => v,
            r => panic!("{r:?}"),
        }
    }
    async fn dump(&self) -> serde_json::Value {
        match self.control(touchbar_control::Command::Dump).await {
            touchbar_control::Reply::Dump(p) => {
                serde_json::from_slice(&fs::read(p).unwrap()).unwrap()
            }
            r => panic!("{r:?}"),
        }
    }
    async fn next(&mut self) -> Peer {
        let event = or(self.fake.next_event(), async {
            async_io::Timer::after(LIMIT).await;
            Err(std::io::ErrorKind::TimedOut.into())
        })
        .await
        .unwrap_or_else(|e| panic!("binary peer {e}: {}", self.log()));
        match event {
            Peer::Received(In::FrameDamage(r)) => self.damage.push(r),
            Peer::Received(In::FrameReady { index, frame }) => {
                if frame == 1 {
                    assert!(self.damage.is_empty(), "first swapchain frame must be full");
                }
                let pixels = self.fake.pixels(index).unwrap();
                self.pending
                    .insert(frame, (pixels, std::mem::take(&mut self.damage)));
            }
            Peer::Presented { frame, .. } => {
                let next = self.pending.remove(&frame).unwrap();
                if self.check_damage {
                    assert!(!next.1.is_empty(), "partial action submitted a full frame");
                    if let Some((before, _)) = &self.last {
                        assert_unchanged_outside(before, &next.0, &next.1);
                    }
                    self.changes.extend(&next.1);
                }
                self.last = Some(next);
            }
            _ => {}
        }
        event
    }
    async fn frame(&mut self, predicate: impl Fn(&serde_json::Value) -> bool) -> Image {
        let at = Instant::now();
        loop {
            assert!(at.elapsed() < LIMIT, "no stable frame: {}", self.log());
            let idle = or(
                async {
                    self.next().await;
                    false
                },
                async {
                    async_io::Timer::after(Duration::from_millis(75)).await;
                    true
                },
            )
            .await;
            if idle && self.last.is_some() {
                let (pixels, _) = self.last.clone().unwrap();
                let status = self.status().await;
                assert!(
                    status["load_issues"].as_array().unwrap().is_empty(),
                    "{status}"
                );
                let instances = status["core"]["instances"].as_array().unwrap();
                let surfaces = status["core"]["surfaces"].as_array().unwrap();
                if surfaces.len() != 1 || instances.is_empty() {
                    continue;
                }
                let s = &surfaces[0];
                if s["profile"]["active"] != "profile-default/default"
                    || !s["profile"]["pending"].is_null()
                    || s["layout_generation"] != s["generation"]
                    || !instances.iter().all(|i| {
                        (i["status"] == "Visible" || i["status"] == "Hidden")
                            && i["in_flight"].is_null()
                            && i["queue_len"] == 0
                    })
                    || !status["core"]["requests"].as_array().unwrap().is_empty()
                    || !predicate(&status)
                {
                    continue;
                }
                let surface = s["info"]["id"].as_u64().unwrap();
                let generation = s["generation"].as_u64().unwrap();
                let dump = self.dump().await;
                if !dump["inputs"].as_array().unwrap().iter().any(|e| {
                    e["input"]["FrameDone"]
                        == serde_json::json!({"surface":surface,"generation":generation})
                }) {
                    continue;
                }
                assert_eq!(status["touchbar"]["path"], "copy");
                assert_eq!(s["info"]["width_px"], WIDTH);
                assert_eq!(s["info"]["height_px"], HEIGHT);
                return Image {
                    pixels,
                    surface,
                    generation,
                };
            }
        }
    }
    async fn layout(&self) -> serde_json::Value {
        let status = self.status().await;
        let s = &status["core"]["surfaces"][0];
        self.dump().await["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find_map(|e| {
                let l = &e["input"]["Layout"];
                (l["surface"] == s["info"]["id"] && l["generation"] == s["generation"])
                    .then(|| l.clone())
            })
            .expect("current layout")
    }
    // Coordinates are taken from the bounded test layout; the wire uses integers.
    #[allow(clippy::cast_possible_truncation)]
    fn touch(&mut self, contact: u32, phase: TouchPhase, x: f64, y: f64) {
        assert!((0.0..=f64::from(WIDTH)).contains(&x) && (0.0..=f64::from(HEIGHT)).contains(&y));
        self.fake
            .send_message(Out::Touch {
                phase,
                contact: std::num::NonZeroU32::new(contact).unwrap(),
                x: x.round() as i32,
                y: y.round() as i32,
            })
            .unwrap();
    }
    async fn wait_status(&self, p: impl Fn(&serde_json::Value) -> bool) {
        let at = Instant::now();
        loop {
            if p(&self.status().await) {
                return;
            }
            assert!(at.elapsed() < LIMIT, "{}", self.log());
            async_io::Timer::after(Duration::from_millis(5)).await;
        }
    }
    fn terminate(&self) {
        rustix::process::kill_process(
            rustix::process::Pid::from_raw(i32::try_from(self.child.0.id()).unwrap()).unwrap(),
            rustix::process::Signal::TERM,
        )
        .unwrap();
    }
    async fn stopped(&mut self) {
        let at = Instant::now();
        loop {
            if let Some(s) = self.child.0.try_wait().unwrap() {
                assert!(s.success(), "{}", self.log());
                return;
            }
            assert!(at.elapsed() < Duration::from_secs(5), "{}", self.log());
            async_io::Timer::after(Duration::from_millis(5)).await;
        }
    }
}
fn golden(image: &Image) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/default-2008x60-dark.png");
    let mut reader = png::Decoder::new(BufReader::new(fs::File::open(path).unwrap()))
        .read_info()
        .unwrap();
    let mut expected = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut expected).unwrap();
    assert_eq!(
        (info.width, info.height, info.color_type),
        (WIDTH, HEIGHT, png::ColorType::Rgba)
    );
    let mut bad = 0;
    for x in 0..WIDTH as usize {
        for y in 0..HEIGHT as usize {
            let src = (y * WIDTH as usize + x) * 4;
            let dst = x * 256 + y * 4;
            let actual = [
                image.pixels[dst + 2],
                image.pixels[dst + 1],
                image.pixels[dst],
                255,
            ];
            if actual
                .iter()
                .zip(&expected[src..src + 4])
                .any(|(a, b)| a.abs_diff(*b) > 2)
            {
                bad += 1;
            }
        }
        assert!(
            image.pixels[x * 256 + 240..(x + 1) * 256]
                .iter()
                .all(|b| *b == 0),
            "pitch padding overwritten"
        );
    }
    assert!(
        f64::from(bad) / (f64::from(WIDTH) * f64::from(HEIGHT)) <= 0.001,
        "{bad} pixels differ from PNG after portrait/BGRA conversion"
    );
}
fn no_panel(s: &serde_json::Value) -> bool {
    s["core"]["surfaces"][0]["presentation"].is_null()
}
fn volume(s: &serde_json::Value) -> f64 {
    s["core"]["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["set"] == "controls/volume")
        .unwrap()["state"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f[0] == "volume")
        .unwrap()[1]["Scalar"]["Number"]
        .as_f64()
        .unwrap()
}
#[test]
#[ignore = "requires EGL/GLES: cargo xtask test-env gpu"]
fn env_gpu_binary_first_frame_matches_golden() {
    futures_lite::future::block_on(async {
        let mut b = Binary::new();
        let f = b
            .frame(|s| no_panel(s) && (volume(s) - 0.5).abs() < 0.001)
            .await;

        golden(&f);
        b.terminate();
        while !matches!(b.next().await, Peer::Disconnected) {}
        b.stopped().await;
    });
}
#[test]
#[ignore = "requires EGL/GLES: cargo xtask test-env gpu"]
fn env_gpu_binary_native_roundtrip() {
    futures_lite::future::block_on(async {
        let mut b = Binary::new();
        let first = b.frame(no_panel).await;
        golden(&first);
        b.fake.set_mode(Mode::Native, ModeReason::Recovery).unwrap();
        b.wait_status(|s| s["touchbar"]["mode"] == "native").await;
        b.fake
            .set_mode(Mode::Display, ModeReason::Recovery)
            .unwrap();
        b.pending.clear();
        b.damage.clear();
        b.last = None;
        let again = b.frame(no_panel).await;
        assert_eq!(first.surface, again.surface);
        assert!(again.generation > first.generation);

        golden(&again);
        b.terminate();
        while !matches!(b.next().await, Peer::Disconnected) {}
        b.stopped().await;
    });
}
#[test]
#[ignore = "requires EGL/GLES: cargo xtask test-env gpu"]
fn env_gpu_binary_revoked_reconnects() {
    futures_lite::future::block_on(async {
        let mut b = Binary::new();
        let first = b.frame(no_panel).await;
        b.fake
            .send_message(Out::Revoked(wire::RevokedReason::Replaced))
            .unwrap();
        b.fake.set_mode(Mode::Display, ModeReason::Owner).unwrap();
        b.pending.clear();
        b.damage.clear();
        b.last = None;
        let again = b.frame(no_panel).await;
        assert_ne!(first.surface, again.surface);

        golden(&again);
        b.terminate();
        while !matches!(b.next().await, Peer::Disconnected) {}
        b.stopped().await;
    });
}
#[test]
#[ignore = "requires EGL/GLES: cargo xtask test-env gpu"]
fn env_gpu_binary_sigterm_releases_keys() {
    futures_lite::future::block_on(async {
        let mut b = Binary::new();
        b.frame(no_panel).await;
        while !b.fake.take_received().contains(&In::WatchKey {
            key: WatchKey::Fn,
            enabled: true,
        }) {
            b.next().await;
        }
        b.fake
            .set_physical_key(WatchKey::Fn, KeyPhase::Pressed)
            .unwrap();
        b.frame(|s| {
            !s["core"]["surfaces"][0]["layers"]
                .as_array()
                .unwrap()
                .is_empty()
        })
        .await;
        let l = b.layout().await;
        let r = &l["hit_map"][4]["rect"];
        b.touch(
            71,
            TouchPhase::Down,
            r["x"].as_f64().unwrap() + r["width"].as_f64().unwrap() / 2.0,
            r["y"].as_f64().unwrap() + r["height"].as_f64().unwrap() / 2.0,
        );
        loop {
            if matches!(
                b.next().await,
                Peer::Received(In::Key {
                    key: SystemKey::F5,
                    phase: KeyPhase::Pressed
                })
            ) {
                break;
            }
        }
        b.terminate();
        let mut released = false;
        loop {
            let event = b.next().await;
            match event {
                Peer::Received(In::Key {
                    key: SystemKey::F5,
                    phase: KeyPhase::Released,
                }) => released = true,
                Peer::Disconnected => {
                    assert!(released, "socket closed before F5 released: {}", b.log());
                    break;
                }
                _ => {}
            }
        }
        b.stopped().await;
    });
}
#[test]
#[ignore = "requires EGL/GLES: cargo xtask test-env gpu"]
fn env_gpu_binary_partial_frame_damage() {
    futures_lite::future::block_on(async {
        let mut b = Binary::new();
        b.frame(no_panel).await;
        b.touch(1, TouchPhase::Down, 80.0, 30.0);
        b.touch(1, TouchPhase::Up, 80.0, 30.0);
        b.frame(|s| !no_panel(s)).await;
        let l = b.layout().await;
        let r = &l["hit_map"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["widget"] == "volume-panel-level")
            .unwrap()["rect"];
        let x = r["x"].as_f64().unwrap();
        let width = r["width"].as_f64().unwrap();
        let y = r["y"].as_f64().unwrap() + r["height"].as_f64().unwrap() / 2.0;
        b.touch(2, TouchPhase::Down, x + width * 0.5, y);
        let before = b.frame(|s| !no_panel(s)).await;
        b.check_damage = true;
        b.touch(2, TouchPhase::Move, x + width * 0.8, y);
        let after = b.frame(|s| volume(s) > 0.7).await;
        assert!(
            !b.changes.is_empty(),
            "volume change unexpectedly became a full frame"
        );
        let bounds = &l["presentation"]["bounds"];
        let low = bounds["x"].as_f64().unwrap().floor();
        let high = (bounds["x"].as_f64().unwrap() + bounds["width"].as_f64().unwrap()).ceil();
        for r in &b.changes {
            assert!(
                f64::from(r.y) >= low && f64::from(r.y) + f64::from(r.height) <= high,
                "damage escaped volume panel: {r:?} vs {bounds}"
            );
        }
        assert_ne!(
            before.pixels, after.pixels,
            "volume change did not alter the image"
        );
        assert_unchanged_outside(&before.pixels, &after.pixels, &b.changes);
        b.check_damage = false;
        b.touch(2, TouchPhase::Up, x + width * 0.8, y);
        b.terminate();
        while !matches!(b.next().await, Peer::Disconnected) {}
        b.stopped().await;
    });
}

fn assert_unchanged_outside(before: &[u8], after: &[u8], damage: &[DamageRect]) {
    for row in 0..WIDTH as usize {
        for col in 0..HEIGHT as usize {
            let offset = row * 256 + col * 4;
            let dirty = damage.iter().any(|r| {
                col >= usize::from(r.x)
                    && col < usize::from(r.x + r.width)
                    && row >= usize::from(r.y)
                    && row < usize::from(r.y + r.height)
            });
            if !dirty {
                assert_eq!(
                    &before[offset..offset + 4],
                    &after[offset..offset + 4],
                    "pixel {col},{row} changed outside damage {damage:?}"
                );
            }
        }
    }
}
