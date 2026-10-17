use std::{cell::RefCell, rc::Rc, sync::Arc};
use touchbar_core::*;
use touchbar_loader::*;
use touchbar_logic_runtime::{MemoryStore, PersistStore};
use touchbar_model::*;
use touchbar_session::{surface::*, *};
fn info() -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(1),
        name: "headless:test".parse().unwrap(),
        kind: SurfaceKind::Headless,
        width_px: 400,
        height_px: 60,
        px_per_mm: 6.2,
        caps: SurfaceCaps::default(),
    }
}
fn session(dir: &std::path::Path, store: MemoryStore) -> (Session, ManualClock) {
    let clock = ManualClock::default();
    let desktop = Rc::new(touchbar_desktop::NullBackend::default());
    let (effects, _) = touchbar_effects::Engine::fake(desktop.clone());
    let options = SessionOptions::new(dir.into(), UserConfig::default());
    (
        Session::new(options, clock.clone(), Box::new(store), desktop, effects).unwrap(),
        clock,
    )
}
#[test]
fn queued_events_do_not_enter_core_before_pump_and_backend_loss_clears_facts() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, _) = session(dir.path(), MemoryStore::default());
    session
        .handle_event(Event::Desktop(touchbar_desktop::BackendEvent::Ready(
            touchbar_desktop::Capabilities::default(),
        )))
        .unwrap();
    session
        .handle_event(Event::Desktop(touchbar_desktop::BackendEvent::Fact(
            "app.id".into(),
            Some(FactValue::Text("test".into())),
        )))
        .unwrap();
    assert!(!session.snapshot().facts.contains_key(&"app.id".into()));
    session.pump().unwrap();
    assert_eq!(
        session.snapshot().facts[&"app.id".into()],
        FactValue::Text("test".into())
    );
    session
        .handle_event(Event::Desktop(touchbar_desktop::BackendEvent::Lost(
            "test disconnect".into(),
        )))
        .unwrap();
    session.pump().unwrap();
    assert!(!session.snapshot().facts.contains_key(&"app.id".into()));
    session.dump().replay().unwrap();
}
struct Delayed {
    info: SurfaceInfo,
    frames: Rc<RefCell<Vec<Arc<PresentedFrame>>>>,
}
impl Surface for Delayed {
    fn info(&self) -> SurfaceInfo {
        self.info.clone()
    }
    fn present(&mut self, f: Arc<PresentedFrame>) -> Result<Presentation, String> {
        self.frames.borrow_mut().push(f);
        Ok(Presentation::Pending)
    }
}
#[test]
fn delayed_presentation_blocks_second_frame_until_done_and_layout_precedes_ack() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, _) = session(dir.path(), MemoryStore::default());
    let frames = Rc::new(RefCell::new(Vec::new()));
    session
        .add_surface(Box::new(Delayed {
            info: info(),
            frames: frames.clone(),
        }))
        .unwrap();
    session.pump().unwrap();
    assert_eq!(frames.borrow().len(), 1);
    let generation = frames.borrow()[0].generation;
    let mut changed = info();
    changed.width_px = 500;
    session
        .handle_event(Event::Input(Input::SurfaceChanged(changed)))
        .unwrap();
    session.pump().unwrap();
    assert_eq!(frames.borrow().len(), 1);
    session
        .handle_event(Event::Surface(SurfaceEvent::Presented {
            surface: SurfaceId(1),
            generation,
        }))
        .unwrap();
    session.pump().unwrap();
    assert_eq!(frames.borrow().len(), 2);
    let trace = session.dump().inputs;
    let layout = trace
        .iter()
        .position(|e| matches!(e.input,Input::Layout{generation:g,..} if g==generation))
        .unwrap();
    let ack = trace
        .iter()
        .position(|e| matches!(e.input,Input::FrameDone{generation:g,..} if g==generation))
        .unwrap();
    assert!(layout < ack);
}
fn plugin(dir: &std::path::Path) -> LoadRequest {
    let plugin = dir.join("plugins/persist");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::copy(touchbar_wasm_fixtures::PERSIST, plugin.join("logic.wasm")).unwrap();
    std::fs::write(plugin.join("plugin.toml"),"id='persist'\nversion='1'\napi='1'\nname='Persist'\ndescription=''\nlogic='logic.wasm'\n[[sets]]\nid='main'\nfile='main.toml'\n[[profiles]]\nid='default'\nfile='default.toml'\n").unwrap();
    std::fs::write(plugin.join("main.toml"),"version=1\nid='main'\nlabel='Value'\npersist=['value']\n[layout]\nkind='slider'\nid='slider'\nvalue_from='value'\non_value_changed='change'\n").unwrap();
    std::fs::write(plugin.join("default.toml"),"version=1\nid='default'\ndefault=true\n[[element]]\nkind='slot'\nid='main'\nitems=['persist/main']\n").unwrap();
    LoadRequest {
        paths: vec![dir.join("plugins")],
        config_file: dir.join("config.toml"),
        bindings_file: dir.join("bindings.toml"),
    }
}
#[test]
fn real_wasm_responses_are_queued_and_shutdown_flushes_debounced_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let mut inspect = store.clone();
    let (mut session, _) = session(dir.path(), store);
    let request = plugin(dir.path());
    let prepared = prepare(request, session.compiler());
    let report = session.load(prepared);
    assert!(report.accepted, "{:?}", report.issues);
    let (surface, handle) = HeadlessSurface::new(info());
    session.add_surface(Box::new(surface)).unwrap();
    session.pump().unwrap();
    let first = session.snapshot().instances[0].clone();
    assert_eq!(first.status, InstanceStatus::Visible);
    assert!(inspect.load(&first.persist_key).unwrap().is_empty());
    let rect = handle.frame().unwrap().layout.hit_map[0].rect;
    for (phase, x) in [
        (TouchPhase::Down, rect.x + 5.0),
        (TouchPhase::Move, rect.x + rect.width * 0.6),
        (TouchPhase::Up, rect.x + rect.width * 0.6),
    ] {
        session
            .handle_event(Event::Input(Input::Touch {
                surface: SurfaceId(1),
                contact: ContactId(std::num::NonZeroU32::new(1).unwrap()),
                phase,
                x,
                y: rect.y + rect.height * 0.5,
            }))
            .unwrap();
        session.pump().unwrap();
    }
    let fields = session.snapshot().instances[0].state.clone();
    let value = fields.iter().find(|(k, _)| k == "value").unwrap().1.clone();
    assert!(matches!(value,StateValue::Scalar(Scalar::Number(v))if v>0.1));
    let trace = session.dump();
    let mut calls = std::collections::BTreeSet::new();
    for entry in &trace.inputs {
        if let Input::LogicUpdate { instance, call, .. } = &entry.input {
            assert!(calls.remove(&(*instance, *call)));
        }
        for output in &entry.outputs {
            if let Output::CallLogic { instance, call, .. } = output {
                assert!(calls.insert((*instance, *call)));
            }
        }
    }
    trace.replay().unwrap();
    session.shutdown().unwrap();
    assert!(session.snapshot().instances.is_empty());
    assert_eq!(
        inspect.load(&first.persist_key).unwrap(),
        vec![
            (
                "label".into(),
                StateValue::Scalar(Scalar::Text("initial".into()))
            ),
            ("value".into(), value)
        ]
    );
}
struct LazyKeys(std::sync::Arc<std::sync::Mutex<Vec<KeyPhase>>>);
impl touchbar_desktop::DesktopBackend for LazyKeys {
    fn name(&self) -> &'static str {
        "lazy-test"
    }
    fn start(
        &mut self,
        _: async_channel::Sender<touchbar_desktop::BackendEvent>,
    ) -> Result<(), touchbar_desktop::DesktopError> {
        Ok(())
    }
    fn command(
        &self,
        command: touchbar_desktop::DesktopCommand,
    ) -> touchbar_desktop::CommandFuture {
        let log = self.0.clone();
        Box::pin(async move {
            let touchbar_desktop::DesktopCommand::Key { phase, .. } = command else {
                return Err(touchbar_desktop::DesktopError::Unsupported);
            };
            log.lock().unwrap().push(phase);
            Ok(())
        })
    }
}
#[test]
fn asynchronous_desktop_key_press_and_release_keep_core_order() {
    let dir = tempfile::tempdir().unwrap();
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let desktop = Rc::new(LazyKeys(log.clone()));
    let (effects, _) = touchbar_effects::Engine::fake(desktop.clone());
    let mut session = Session::new(
        SessionOptions::new(dir.path().into(), UserConfig::default()),
        ManualClock::default(),
        Box::<MemoryStore>::default(),
        desktop,
        effects,
    )
    .unwrap();
    let request = plugin(dir.path());
    std::fs::write(
        dir.path().join("plugins/persist/main.toml"),
        "version=1\nid='main'\nlabel='Key'\n[layout]\nkind='key'\nid='key'\nkey='f1'\n",
    )
    .unwrap();
    assert!(session.load(prepare(request, session.compiler())).accepted);
    let (surface, handle) = HeadlessSurface::new(info());
    session.add_surface(Box::new(surface)).unwrap();
    let mut caps = touchbar_desktop::Capabilities::default();
    caps.commands.insert(touchbar_desktop::CommandKind::Key);
    session
        .handle_event(Event::Desktop(touchbar_desktop::BackendEvent::Ready(caps)))
        .unwrap();
    session.pump().unwrap();
    let rect = handle.frame().unwrap().layout.hit_map[0].rect;
    for phase in [TouchPhase::Down, TouchPhase::Up] {
        session
            .handle_event(Event::Input(Input::Touch {
                surface: SurfaceId(1),
                contact: ContactId(std::num::NonZeroU32::new(1).unwrap()),
                phase,
                x: rect.x + rect.width * 0.5,
                y: rect.y + rect.height * 0.5,
            }))
            .unwrap();
        session.pump().unwrap();
    }
    futures_lite::future::block_on(futures_lite::future::or(
        async {
            session.next_executor().await;
            session.next_executor().await;
        },
        async {
            async_io::Timer::after(std::time::Duration::from_secs(2)).await;
            panic!("key commands stalled")
        },
    ));
    assert_eq!(
        *log.lock().unwrap(),
        [KeyPhase::Pressed, KeyPhase::Released]
    );
}
struct FailOnceStore {
    inner: MemoryStore,
    remaining: usize,
}
impl PersistStore for FailOnceStore {
    fn load(&mut self, key: &PersistKey) -> Result<StatePatch, touchbar_logic_runtime::StoreError> {
        self.inner.load(key)
    }
    fn write(
        &mut self,
        key: &PersistKey,
        fields: &StatePatch,
    ) -> Result<(), touchbar_logic_runtime::StoreError> {
        if self.remaining > 0 {
            self.remaining -= 1;
            Err(touchbar_logic_runtime::StoreError(
                "temporary disk failure".into(),
            ))
        } else {
            self.inner.write(key, fields)
        }
    }
}
#[test]
fn shutdown_retries_failed_write_even_after_core_marked_delta_emitted() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let mut inspect = store.clone();
    let desktop = Rc::new(touchbar_desktop::NullBackend::default());
    let (effects, _) = touchbar_effects::Engine::fake(desktop.clone());
    let clock = ManualClock::default();
    let mut session = Session::new(
        SessionOptions::new(dir.path().into(), UserConfig::default()),
        clock.clone(),
        Box::new(FailOnceStore {
            inner: store,
            remaining: 1,
        }),
        desktop,
        effects,
    )
    .unwrap();
    let request = plugin(dir.path());
    assert!(session.load(prepare(request, session.compiler())).accepted);
    let (surface, _) = HeadlessSurface::new(info());
    session.add_surface(Box::new(surface)).unwrap();
    session.pump().unwrap();
    let key = session.snapshot().instances[0].persist_key.clone();
    clock.advance(1000);
    session.handle_event(Event::Wake).unwrap();
    session.pump().unwrap();
    assert!(inspect.load(&key).unwrap().is_empty());
    assert!(
        session
            .dump()
            .replay()
            .unwrap()
            .core
            .pending_persist()
            .is_empty()
    );
    session.shutdown().unwrap();
    assert!(!inspect.load(&key).unwrap().is_empty());
}
#[test]
fn appearance_scheme_redraws_even_without_a_logic_update() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, _) = session(dir.path(), MemoryStore::default());
    let (surface, handle) = HeadlessSurface::new(info());
    session.add_surface(Box::new(surface)).unwrap();
    session.pump().unwrap();
    let before = handle.frame().unwrap().display_list.clone();
    session
        .handle_event(Event::Input(Input::Fact {
            name: "appearance.scheme".into(),
            value: Some(FactValue::Text("light".into())),
        }))
        .unwrap();
    session.pump().unwrap();
    assert_ne!(handle.frame().unwrap().display_list, before);
}
struct AdvancingSurface {
    clock: ManualClock,
}
impl Surface for AdvancingSurface {
    fn info(&self) -> SurfaceInfo {
        info()
    }
    fn present(&mut self, _: Arc<PresentedFrame>) -> Result<Presentation, String> {
        self.clock.advance(9);
        Ok(Presentation::Complete)
    }
}
#[test]
fn pump_yields_between_native_steps_instead_of_starving_the_event_loop() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, clock) = session(dir.path(), MemoryStore::default());
    session
        .add_surface(Box::new(AdvancingSurface { clock }))
        .unwrap();
    let result = session.pump().unwrap();
    assert!(result.pending, "pump ignored its time slice");
    assert!(!session.pump().unwrap().pending);
}
#[test]
fn shutdown_does_not_execute_inputs_that_have_not_entered_core() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let mut inspect = store.clone();
    let (mut session, _) = session(dir.path(), store);
    let request = plugin(dir.path());
    assert!(session.load(prepare(request, session.compiler())).accepted);
    let (surface, handle) = HeadlessSurface::new(info());
    session.add_surface(Box::new(surface)).unwrap();
    session.pump().unwrap();
    let key = session.snapshot().instances[0].persist_key.clone();
    let rect = handle.frame().unwrap().layout.hit_map[0].rect;
    for (phase, x) in [
        (TouchPhase::Down, rect.x + 5.0),
        (TouchPhase::Move, rect.x + rect.width * 0.6),
    ] {
        session
            .handle_event(Event::Input(Input::Touch {
                surface: SurfaceId(1),
                contact: ContactId(std::num::NonZeroU32::new(1).unwrap()),
                phase,
                x,
                y: rect.y + rect.height * 0.5,
            }))
            .unwrap();
    }
    session.shutdown().unwrap();
    let fields = inspect.load(&key).unwrap();
    assert!(fields.contains(&("value".into(), StateValue::Scalar(Scalar::Number(0.0)))));
}
#[derive(Clone, Default)]
struct StepClock(Rc<std::cell::Cell<u64>>);
impl Clock for StepClock {
    fn now(&self) -> Timestamp {
        let now = self.0.get();
        self.0.set(now.saturating_add(9));
        Timestamp(now)
    }
}
#[test]
fn shutdown_preserves_a_persist_output_not_yet_delivered_to_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let mut inspect = store.clone();
    let clock = StepClock::default();
    let desktop = Rc::new(touchbar_desktop::NullBackend::default());
    let (effects, _) = touchbar_effects::Engine::fake(desktop.clone());
    let mut session = Session::new(
        SessionOptions::new(dir.path().into(), UserConfig::default()),
        clock.clone(),
        Box::new(store),
        desktop,
        effects,
    )
    .unwrap();
    while session.has_pending() {
        session.pump().unwrap();
    }
    let request = plugin(dir.path());
    assert!(session.load(prepare(request, session.compiler())).accepted);
    let (surface, _) = HeadlessSurface::new(info());
    session.add_surface(Box::new(surface)).unwrap();
    while session.has_pending() {
        session.pump().unwrap();
    }
    let key = session.snapshot().instances[0].persist_key.clone();
    clock.0.set(100_000);
    session.handle_event(Event::Wake).unwrap();
    session.pump().unwrap();
    session.pump().unwrap();
    assert!(inspect.load(&key).unwrap().is_empty());
    assert!(
        session
            .dump()
            .replay()
            .unwrap()
            .core
            .pending_persist()
            .is_empty()
    );
    assert!(session.has_pending());
    session.shutdown().unwrap();
    assert!(!inspect.load(&key).unwrap().is_empty());
}
