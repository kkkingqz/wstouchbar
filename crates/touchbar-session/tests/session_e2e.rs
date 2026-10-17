mod support;
use std::{fs, path::Path};
use support::*;
use touchbar_core::*;
use touchbar_model::*;
#[test]
fn session_starts_with_default_profile() {
    let f = Fixture::new(2008, 60, false);
    assert_eq!(
        f.session.snapshot().surfaces[0].scene.profile,
        Some("profile-default/default".parse().unwrap())
    );
    assert!(
        f.session
            .snapshot()
            .instances
            .iter()
            .all(|i| i.status == InstanceStatus::Visible)
    );
    cpu_golden("default-2008x60-dark", &f.frame(0));
    f.checked();
}
#[test]
fn external_volume_moves_slider() {
    let mut f = Fixture::new(2008, 60, false);
    f.panel(0);
    let before = f.frame(0).display_list.clone();
    f.audio(AudioTarget::DefaultSink, 0.8, false);
    assert_ne!(before, f.frame(0).display_list);
    assert_eq!(
        f.state(0, "volume", "volume"),
        StateValue::Scalar(Scalar::Number(0.8))
    );
    f.checked();
}
#[test]
fn mic_tap_toggles() {
    let mut f = Fixture::new(2008, 60, false);
    let before = f.frame(0).display_list.clone();
    f.tap(0, "microphone-toggle");
    assert!(f.effects.started().iter().any(|(_, e)| matches!(
        e,
        Effect::AudioSetMute {
            target: AudioTarget::DefaultSource,
            value: MuteValue::Toggle
        }
    )));
    assert_eq!(
        f.state(0, "microphone", "microphone_muted"),
        StateValue::Scalar(Scalar::Boolean(true))
    );
    assert_ne!(before, f.frame(0).display_list);
    f.checked();
}
#[test]
fn volume_panel_hold_opens_transient() {
    let mut f = Fixture::new(2008, 60, false);
    let r = f.rect(0, "volume-panel");
    f.touch(
        0,
        1,
        TouchPhase::Down,
        r.x + r.width * 0.5,
        r.y + r.height * 0.5,
    );
    f.advance(419);
    assert!(f.session.snapshot().surfaces[0].presentation.is_none());
    f.advance(1);
    assert!(f.session.snapshot().surfaces[0].presentation.is_some());
    assert!(!f.frame(0).layout.hit_map.is_empty());
    f.touch(
        0,
        1,
        TouchPhase::Up,
        r.x + r.width * 0.5,
        r.y + r.height * 0.5,
    );
    assert!(f.session.snapshot().surfaces[0].presentation.is_none());
    f.checked();
}
#[test]
fn fn_shows_fkeys_layer() {
    let mut f = Fixture::new(2008, 60, false);
    f.send(Input::Key {
        source: KeySource::Surface(SurfaceId(1)),
        code: WatchKey::Fn,
        phase: KeyPhase::Pressed,
    });
    assert_eq!(f.frame(0).layout.hit_map.len(), 12);
    let hit = f.frame(0).layout.hit_map[4].rect;
    f.touch(
        0,
        1,
        TouchPhase::Down,
        hit.x + hit.width * 0.5,
        hit.y + hit.height * 0.5,
    );
    f.touch(
        0,
        1,
        TouchPhase::Up,
        hit.x + hit.width * 0.5,
        hit.y + hit.height * 0.5,
    );
    assert_eq!(
        f.handles[0].keys(),
        vec![
            (SystemKey::F5, KeyPhase::Pressed),
            (SystemKey::F5, KeyPhase::Released)
        ]
    );
    f.send(Input::Key {
        source: KeySource::Surface(SurfaceId(1)),
        code: WatchKey::Fn,
        phase: KeyPhase::Released,
    });
    f.checked();
}
#[test]
fn broken_reload_keeps_catalog() {
    let mut f = Fixture::new(2008, 60, false);
    let old = f.session.catalog().clone();
    let ids = f
        .session
        .snapshot()
        .instances
        .iter()
        .map(|i| i.id)
        .collect::<Vec<_>>();
    fs::write(f.file("controls/sets/volume.toml"), "broken=").unwrap();
    assert!(!f.reload().accepted);
    f.settle();
    assert_eq!(f.session.catalog(), &old);
    assert_eq!(
        f.session
            .snapshot()
            .instances
            .iter()
            .map(|i| i.id)
            .collect::<Vec<_>>(),
        ids
    );
    assert!(!f.session.status().load_issues.is_empty());
    f.checked();
}
#[test]
fn reload_new_revision_recreates() {
    let mut f = Fixture::new(2008, 60, false);
    f.send(Input::Key {
        source: KeySource::Surface(SurfaceId(1)),
        code: WatchKey::Fn,
        phase: KeyPhase::Pressed,
    });
    let old = f.session.snapshot().instances;
    let path = f.file("controls/sets/volume.toml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        path,
        text.replace("label = \"Volume\"", "label = \"Sound\""),
    )
    .unwrap();
    assert!(f.reload().accepted);
    f.settle();
    // A revision covers the entire plugin, so all controls users recreate.
    for previous in old {
        let now = f
            .session
            .snapshot()
            .instances
            .into_iter()
            .find(|i| i.set == previous.set)
            .unwrap();
        if previous.set.plugin.as_str() == "controls" {
            assert_ne!(now.id, previous.id);
        } else {
            assert_eq!(now.id, previous.id);
        }
    }
    f.checked();
}
#[test]
fn two_surfaces_independent() {
    let mut f = Fixture::new(2008, 60, false);
    let shared_sources = f.effects.sources();
    f.add_surface("window:test", 1200, 160);
    f.settle();
    assert_eq!(f.effects.sources(), shared_sources);
    let other = f.frame(1).display_list.clone();
    f.panel(0);
    let snapshot = f.session.snapshot();
    assert!(snapshot.surfaces[0].presentation.is_some());
    assert!(snapshot.surfaces[1].presentation.is_none());
    assert_eq!(f.frame(1).display_list, other);
    f.audio(AudioTarget::DefaultSink, 0.8, false);
    assert_eq!(
        f.state(0, "volume", "volume"),
        f.state(1, "volume", "volume")
    );
    f.send(Input::Key {
        source: KeySource::Surface(SurfaceId(1)),
        code: WatchKey::Fn,
        phase: KeyPhase::Pressed,
    });
    assert!(f.session.snapshot().surfaces[1].layers.is_empty());
    let ids = f
        .session
        .snapshot()
        .instances
        .iter()
        .map(|i| i.id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), f.session.snapshot().instances.len());
    f.checked();
}
#[test]
fn fault_writes_dump() {
    let mut f = Fixture::component(2008, 60, false, Path::new(touchbar_wasm_fixtures::TRAP));
    f.panel(0);
    let r = f.rect(0, "volume-panel-level");
    f.touch(
        0,
        2,
        TouchPhase::Down,
        r.x + r.width * 0.5,
        r.y + r.height * 0.5,
    );
    assert!(
        f.session
            .snapshot()
            .instances
            .iter()
            .any(|i| i.set.id() == "volume" && i.status == InstanceStatus::Faulted)
    );
    let dumps = fs::read_dir(f.dir.path().join("state/touchbar/dumps"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(dumps.len(), 1);
    let dump: ReplayDump = serde_json::from_slice(&fs::read(dumps[0].path()).unwrap()).unwrap();
    dump.replay().unwrap();
    f.checked();
}
#[test]
fn volume_drag_reaches_audio() {
    let mut f = Fixture::new(2008, 60, false);
    f.panel(0);
    f.effects.set_rule(
        "audio.set-volume",
        touchbar_effects::FakeRule {
            delay_ms: 1000,
            result: Ok(EffectResult::None),
        },
    );
    let r = f.rect(0, "volume-panel-level");
    let y = r.y + r.height * 0.5;
    f.touch(0, 2, TouchPhase::Down, r.x + r.width * 0.5, y);
    for index in 1..=10 {
        f.touch(
            0,
            2,
            TouchPhase::Move,
            r.x + r.width * (0.5 + 0.04 * index as f32),
            y,
        );
    }
    f.touch(0, 2, TouchPhase::Up, r.x + r.width * 0.9, y);
    let started = || {
        f.effects
            .started()
            .into_iter()
            .filter_map(|(_, effect)| match effect {
                Effect::AudioSetVolume { level, .. } => Some(level),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(started().len(), 1, "first write must still be executing");
    assert_eq!(
        f.session
            .snapshot()
            .requests
            .iter()
            .filter(|r| matches!(r.effect, Effect::AudioSetVolume { .. }))
            .count(),
        2
    );
    futures_lite::future::block_on(async_io::Timer::after(std::time::Duration::from_millis(
        1020,
    )));
    f.settle();
    let writes = f
        .effects
        .started()
        .into_iter()
        .filter_map(|(_, effect)| match effect {
            Effect::AudioSetVolume { level, .. } => Some(level),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(writes.len(), 2);
    assert!((writes[1] - 0.9).abs() < 0.02, "{writes:?}");
    futures_lite::future::block_on(async_io::Timer::after(std::time::Duration::from_millis(
        1020,
    )));
    f.settle();
    f.checked();
}
#[test]
fn profile_conflict_reported() {
    let mut f = Fixture::new(2008, 60, false);
    let other = f.file("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("plugin.toml"),"id='other'\nversion='1'\napi='1'\nname='Other'\ndescription=''\n[[profiles]]\nid='default'\nfile='default.toml'\n").unwrap();
    fs::write(other.join("default.toml"),"version=1\nid='default'\ndefault=true\n[[element]]\nkind='slot'\nid='keys'\nitems=['system/fkeys']\n").unwrap();
    assert!(f.reload().accepted);
    f.settle();
    let status = f.session.status();
    let surface = &status.core.surfaces[0];
    assert!(surface.scene.profile.is_none());
    assert!(surface.resolution.steps.iter().any(|step| step.conflict));
    assert!(
        serde_json::to_string(&status)
            .unwrap()
            .contains("other/default")
    );
    f.checked();
}
fn install_persist(f: &mut Fixture) -> PersistKey {
    let plugin = f.file("persist");
    fs::create_dir(&plugin).unwrap();
    fs::copy(touchbar_wasm_fixtures::PERSIST, plugin.join("logic.wasm")).unwrap();
    fs::write(plugin.join("plugin.toml"),"id='persist'\nversion='1'\napi='1'\nname='Persist'\ndescription=''\nlogic='logic.wasm'\n[[sets]]\nid='main'\nfile='main.toml'\n[[profiles]]\nid='default'\nfile='default.toml'\n").unwrap();
    fs::write(plugin.join("main.toml"),"version=1\nid='main'\nlabel='Value'\npersist=['value']\n[layout]\nkind='slider'\nid='slider'\nvalue_from='value'\non_value_changed='change'\n").unwrap();
    fs::write(plugin.join("default.toml"),"version=1\nid='default'\ndefault=true\n[[element]]\nkind='slot'\nid='main'\nitems=['persist/main']\n").unwrap();
    fs::write(f.request().config_file, "disabled=['profile-default']\n").unwrap();
    assert!(f.reload().accepted);
    f.settle();
    let r = f.rect(0, "slider");
    f.touch(
        0,
        1,
        TouchPhase::Down,
        r.x + r.width * 0.25,
        r.y + r.height * 0.5,
    );
    f.touch(
        0,
        1,
        TouchPhase::Move,
        r.x + r.width * 0.73,
        r.y + r.height * 0.5,
    );
    f.touch(
        0,
        1,
        TouchPhase::Up,
        r.x + r.width * 0.73,
        r.y + r.height * 0.5,
    );
    f.session
        .snapshot()
        .instances
        .into_iter()
        .find(|i| i.set.plugin.as_str() == "persist")
        .unwrap()
        .persist_key
}
#[test]
fn shutdown_flushes_persist() {
    use touchbar_logic_runtime::PersistStore;
    let mut f = Fixture::new(2008, 60, false);
    let key = install_persist(&mut f);
    let expected = f.state(0, "main", "value");
    let mut store = touchbar_logic_runtime::FsStore::new(f.dir.path().join("state"));
    assert!(store.load(&key).unwrap().is_empty());
    f.session.shutdown().unwrap();
    assert_eq!(
        store.load(&key).unwrap(),
        vec![
            (
                "label".into(),
                StateValue::Scalar(Scalar::Text("initial".into()))
            ),
            ("value".into(), expected)
        ]
    );
    f.checked();
}
#[test]
fn persist_survives_restart() {
    use std::rc::Rc;
    let mut f = Fixture::new(2008, 60, false);
    let _key = install_persist(&mut f);
    let expected = f.state(0, "main", "value");
    let info = f.session.snapshot().surfaces[0].info.clone();
    f.advance(1000);
    f.session.shutdown().unwrap();
    let desktop = Rc::new(touchbar_desktop::NullBackend::default());
    let (effects, _) = touchbar_effects::Engine::fake(desktop.clone());
    let mut restarted = touchbar_session::Session::new_with_painter(
        touchbar_session::SessionOptions::new(
            f.dir.path().join("state"),
            touchbar_loader::UserConfig::default(),
        ),
        touchbar_session::ManualClock::default(),
        Box::new(touchbar_logic_runtime::FsStore::new(
            f.dir.path().join("state"),
        )),
        desktop,
        effects,
        fonts(),
    )
    .unwrap();
    let report = restarted.load(touchbar_loader::prepare(f.request(), restarted.compiler()));
    assert!(report.accepted, "{report:?}");
    let (surface, _) = touchbar_session::surface::HeadlessSurface::new(info);
    restarted.add_surface(Box::new(surface)).unwrap();
    restarted.pump().unwrap();
    let state = restarted
        .snapshot()
        .instances
        .into_iter()
        .find(|i| i.set.plugin.as_str() == "persist")
        .unwrap()
        .state;
    assert!(state.contains(&("value".into(), expected)));
    restarted.dump().replay().unwrap();
}
fn keyframes(
    width: u32,
    height: u32,
    light: bool,
) -> Vec<(
    String,
    std::sync::Arc<touchbar_session::surface::PresentedFrame>,
)> {
    let mut f = Fixture::new(width, height, light);
    let mut frames = vec![("default".into(), f.frame(0))];
    f.panel(0);
    frames.push(("panel".into(), f.frame(0)));
    f.audio(AudioTarget::DefaultSink, 0.8, false);
    frames.push(("external-volume".into(), f.frame(0)));
    f.touch(0, 1, TouchPhase::Up, 0.0, 0.0);
    if f.frame(0)
        .layout
        .hit_map
        .iter()
        .any(|h| h.widget.as_str() == "microphone-toggle")
    {
        f.tap(0, "microphone-toggle");
        frames.push(("muted-mic".into(), f.frame(0)));
    } else {
        // Fixed-width slots cannot all fit this aspect ratio; layout hides mic.
        assert_eq!((width, height), (1200, 160));
    }
    f.send(Input::Key {
        source: KeySource::Surface(SurfaceId(1)),
        code: WatchKey::Fn,
        phase: KeyPhase::Pressed,
    });
    frames.push(("fkeys".into(), f.frame(0)));
    f.checked();
    frames
}
#[test]
fn session_keyframes_have_exact_cpu_goldens_at_three_sizes_and_two_themes() {
    for (w, h) in [(2008, 60), (2170, 60), (1200, 160)] {
        for (scheme, light) in [("dark", false), ("light", true)] {
            for (name, frame) in keyframes(w, h, light) {
                cpu_golden(&format!("{name}-{w}x{h}-{scheme}"), &frame);
            }
        }
    }
}
#[test]
#[ignore = "requires EGL (Mesa llvmpipe)"]
fn env_gpu_session_keyframes_match_png_goldens() {
    with_gpu(|| {
        session_starts_with_default_profile();
        volume_drag_reaches_audio();
        external_volume_moves_slider();
        mic_tap_toggles();
        volume_panel_hold_opens_transient();
        fn_shows_fkeys_layer();
        reload_new_revision_recreates();
        broken_reload_keeps_catalog();
        profile_conflict_reported();
        persist_survives_restart();
        shutdown_flushes_persist();
        fault_writes_dump();
        two_surfaces_independent();
        for (w, h) in [(2008, 60), (2170, 60), (1200, 160)] {
            for (scheme, light) in [("dark", false), ("light", true)] {
                for (name, frame) in keyframes(w, h, light) {
                    let output = frame.pixels.as_ref().expect("session GL output");
                    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join(format!("tests/golden/{name}-{w}x{h}-{scheme}.png"));
                    if std::env::var_os("UPDATE_GOLDEN").as_deref()
                        == Some(std::ffi::OsStr::new("1"))
                    {
                        fs::create_dir_all(path.parent().unwrap()).unwrap();
                        output.save_png(&path).unwrap();
                    }
                    let mut reader =
                        png::Decoder::new(std::io::BufReader::new(fs::File::open(&path).unwrap()))
                            .read_info()
                            .unwrap();
                    let mut expected = vec![0; reader.output_buffer_size().unwrap()];
                    let info = reader.next_frame(&mut expected).unwrap();
                    assert_eq!(
                        (info.width, info.height, info.color_type),
                        (w, h, png::ColorType::Rgba)
                    );
                    assert_eq!(expected.len(), output.pixels.len());
                    let bad = expected
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(output.pixels.as_chunks::<4>().0.iter())
                        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(a, b)| a.abs_diff(*b) > 2))
                        .count();
                    assert!(
                        (bad as f64) / (f64::from(w) * f64::from(h)) <= 0.001,
                        "{}: {bad} changed pixels",
                        path.display()
                    );
                }
            }
        }
    });
}
