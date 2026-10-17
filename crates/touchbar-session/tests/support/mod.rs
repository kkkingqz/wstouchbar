#![allow(dead_code)]
use std::{fs, path::Path, rc::Rc, sync::Arc};
use touchbar_core::*;
use touchbar_desktop::{BackendEvent, DesktopBackend, FakeBackend};
use touchbar_effects::{Engine, FakeControl};
use touchbar_loader::*;
use touchbar_model::*;
use touchbar_session::{surface::*, *};
pub fn fonts() -> touchbar_ui::CpuPainter {
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_font_data(include_bytes!("../../../../assets/fonts/test/InterVariable.ttf").to_vec());
    db.set_sans_serif_family("Inter Variable");
    touchbar_ui::CpuPainter::new(cosmic_text::FontSystem::new_with_locale_and_db(
        "en-US".into(),
        db,
    ))
}
pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub session: Session,
    pub clock: ManualClock,
    pub desktop: FakeBackend,
    pub effects: FakeControl,
    pub handles: Vec<HeadlessHandle>,
    events: async_channel::Receiver<BackendEvent>,
}
impl Fixture {
    pub fn new(width: u32, height: u32, light: bool) -> Self {
        Self::component(
            width,
            height,
            light,
            Path::new(touchbar_wasm_fixtures::CONTROLS),
        )
    }
    pub fn component(width: u32, height: u32, light: bool, component: &Path) -> Self {
        let dir = tempfile::tempdir().unwrap();
        xtask::assemble_plugins(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .as_path(),
            &dir.path().join("plugins"),
            component,
        )
        .unwrap();
        let mut desktop = FakeBackend::default();
        if light {
            desktop
                .set_fact(
                    "appearance.scheme".into(),
                    Some(FactValue::Text("light".into())),
                )
                .unwrap();
        }
        let (tx, events) = async_channel::bounded(512);
        desktop.start(tx).unwrap();
        let backend: Rc<dyn DesktopBackend> = Rc::new(desktop.clone());
        let (engine, effects) = Engine::fake(backend.clone());
        let clock = ManualClock::default();
        let mut options = SessionOptions::new(dir.path().join("state"), UserConfig::default());
        options.motion = touchbar_ui::MotionPolicy::Disabled;
        if GPU.get() {
            options.gl = touchbar_gl::Backend::Surfaceless;
        }
        let session = Session::new_with_painter(
            options,
            clock.clone(),
            Box::new(touchbar_logic_runtime::FsStore::new(
                dir.path().join("state"),
            )),
            backend,
            engine,
            fonts(),
        )
        .unwrap();
        let mut this = Self {
            dir,
            session,
            clock,
            desktop,
            effects,
            handles: vec![],
            events,
        };
        assert!(this.reload().accepted);
        this.add_surface("touchbar", width, height);
        this.settle();
        this
    }
    pub fn request(&self) -> LoadRequest {
        LoadRequest {
            paths: vec![self.dir.path().join("plugins")],
            config_file: self.dir.path().join("config.toml"),
            bindings_file: self.dir.path().join("bindings.toml"),
        }
    }
    pub fn reload(&mut self) -> LoadReport {
        self.session
            .load(prepare(self.request(), self.session.compiler()))
    }
    pub fn add_surface(&mut self, name: &str, width: u32, height: u32) {
        let id = SurfaceId(u64::try_from(self.handles.len()).unwrap() + 1);
        let kind = if name == "touchbar" {
            SurfaceKind::Touchbar
        } else {
            SurfaceKind::Window
        };
        let info = SurfaceInfo {
            id,
            name: name.parse().unwrap(),
            kind,
            width_px: width,
            height_px: height,
            px_per_mm: 6.2,
            caps: SurfaceCaps {
                watch_keys: kind == SurfaceKind::Touchbar,
                inject_keys: kind == SurfaceKind::Touchbar,
                ..SurfaceCaps::default()
            },
        };
        let (surface, handle) = HeadlessSurface::new(info);
        self.handles.push(handle);
        let surface: Box<dyn Surface> = if GPU.get() {
            Box::new(GpuSurface {
                inner: surface,
                last: None,
            })
        } else {
            Box::new(surface)
        };
        self.session.add_surface(surface).unwrap();
    }
    pub fn settle(&mut self) {
        futures_lite::future::block_on(async {
            for _ in 0..5000 {
                while let Ok(event) = self.events.try_recv() {
                    self.session.handle_event(Event::Desktop(event)).unwrap();
                }
                self.session.pump().unwrap();
                if let Some(event) =
                    futures_lite::future::poll_once(self.session.next_executor()).await
                {
                    self.session.handle_event(event).unwrap();
                    continue;
                }
                if !self.session.has_pending() {
                    return;
                }
            }
            panic!("session did not settle");
        });
    }
    pub fn send(&mut self, input: Input) {
        self.session.handle_event(Event::Input(input)).unwrap();
        self.settle();
    }
    pub fn advance(&mut self, ms: u64) {
        self.clock.advance(ms);
        self.session.handle_event(Event::Wake).unwrap();
        self.settle();
    }
    pub fn frame(&self, surface: usize) -> Arc<PresentedFrame> {
        self.handles[surface].frame().unwrap()
    }
    pub fn rect(&self, surface: usize, widget: &str) -> Rect {
        self.frame(surface)
            .layout
            .hit_map
            .iter()
            .find(|h| h.widget.as_str() == widget)
            .unwrap_or_else(|| panic!("no {widget}: {:?}", self.frame(surface).layout.hit_map))
            .rect
    }
    pub fn touch(&mut self, surface: usize, contact: u32, phase: TouchPhase, x: f32, y: f32) {
        self.send(Input::Touch {
            surface: SurfaceId(u64::try_from(surface).unwrap() + 1),
            contact: ContactId::new(contact).unwrap(),
            phase,
            x,
            y,
        });
    }
    pub fn tap(&mut self, surface: usize, widget: &str) {
        let r = self.rect(surface, widget);
        let (x, y) = (r.x + r.width * 0.5, r.y + r.height * 0.5);
        self.touch(surface, 1, TouchPhase::Down, x, y);
        self.touch(surface, 1, TouchPhase::Up, x, y);
    }
    pub fn panel(&mut self, surface: usize) {
        let r = self.rect(surface, "volume-panel");
        self.touch(
            surface,
            1,
            TouchPhase::Down,
            r.x + r.width * 0.5,
            r.y + r.height * 0.5,
        );
        self.advance(420);
    }
    pub fn state(&self, surface: usize, set: &str, field: &str) -> StateValue {
        self.session
            .snapshot()
            .instances
            .iter()
            .find(|i| {
                i.surface == SurfaceId(u64::try_from(surface).unwrap() + 1) && i.set.id() == set
            })
            .unwrap()
            .state
            .iter()
            .find(|(k, _)| k == field)
            .unwrap_or_else(|| panic!("no {field}"))
            .1
            .clone()
    }
    pub fn audio(&mut self, target: AudioTarget, volume: f64, muted: bool) {
        self.effects
            .emit(
                &SubscriptionKind::AudioWatch { target },
                SubscriptionData::Audio(AudioState {
                    available: true,
                    volume,
                    muted,
                    name: "Test audio".into(),
                }),
            )
            .unwrap();
        self.settle();
    }
    pub fn checked(&self) {
        let dump = self.session.dump();
        let mut core = Core::restore(dump.checkpoint.clone()).unwrap();
        let mut checker = touchbar_testkit::InvariantChecker::new(core.snapshot());
        for entry in &dump.inputs {
            assert_eq!(core.handle(entry.at, entry.input.clone()), entry.outputs);
            checker.observe(entry, &core.snapshot()).unwrap();
        }
        assert_eq!(core.snapshot(), self.session.snapshot());
        dump.replay().unwrap();
    }
    pub fn file(&self, path: &str) -> std::path::PathBuf {
        self.dir.path().join("plugins").join(path)
    }
}
pub fn projection(frame: &PresentedFrame) -> serde_json::Value {
    let resources = frame
        .display_list
        .resources
        .iter()
        .map(|(k, r)| {
            let value = match r {
                touchbar_ui::Resource::Raster { width, height, .. } => {
                    serde_json::json!({"kind":"raster","width":width,"height":height})
                }
                touchbar_ui::Resource::Shader { body } => {
                    serde_json::json!({"kind":"shader","body":body})
                }
            };
            (k.clone(), value)
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    serde_json::json!({"clear":frame.display_list.clear,"commands":frame.display_list.commands,"resources":resources,"layout":frame.layout})
}
pub fn cpu_golden(name: &str, frame: &PresentedFrame) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.json"));
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(&projection(frame)).unwrap()
    );
    if std::env::var_os("UPDATE_GOLDEN").as_deref() == Some(std::ffi::OsStr::new("1")) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &text).unwrap();
    }
    assert_eq!(
        fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("missing {}; run UPDATE_GOLDEN=1", path.display())),
        text,
        "{}",
        path.display()
    );
}
thread_local! { static GPU:std::cell::Cell<bool>=const {std::cell::Cell::new(false)}; }
pub fn with_gpu(work: impl FnOnce()) {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            GPU.set(self.0);
        }
    }
    let _reset = Reset(GPU.replace(true));
    work();
}
struct GpuSurface {
    inner: HeadlessSurface,
    last: Option<touchbar_gl::RenderedFrame>,
}
impl Surface for GpuSurface {
    fn info(&self) -> SurfaceInfo {
        self.inner.info()
    }
    fn needs_pixels(&self) -> bool {
        true
    }
    fn present(&mut self, mut frame: Arc<PresentedFrame>) -> Result<Presentation, String> {
        if let Some(pixels) = &frame.pixels {
            self.last = Some(pixels.clone());
        } else {
            Arc::make_mut(&mut frame).pixels = self.last.clone();
        }
        assert!(frame.pixels.is_some(), "GPU surface has no initial pixels");
        self.inner.present(frame)
    }
    fn send_key(&mut self, key: SystemKey, phase: KeyPhase) -> Result<(), String> {
        self.inner.send_key(key, phase)
    }
}
