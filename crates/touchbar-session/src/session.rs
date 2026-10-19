use crate::{
    clock::*,
    diagnostics::{Diagnostics, DumpStore, Level, Record},
    surface::*,
};
use anyhow::{Result, anyhow, ensure};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    future::Future,
    path::PathBuf,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use touchbar_core::*;
use touchbar_desktop::{BackendEvent, BackendMonitor, CommandKind, DesktopBackend, DesktopCommand};
use touchbar_effects::Engine;
use touchbar_loader::{LoadIssue, LoadReport, Loader, PreparedLoad, UserConfig};
use touchbar_logic_runtime::{Compiler, PersistStore, Runtime, RuntimeConfig};
use touchbar_logic_types::LogicLimits;
use touchbar_model::*;
use touchbar_scene::{Documents, SceneRenderer};
use touchbar_ui::{CpuPainter, MotionPolicy, Theme};
const QUEUE_LIMIT: usize = 16384;
const PUMP_LIMIT: usize = 4096;
#[derive(Clone)]
pub struct SessionOptions {
    pub config: UserConfig,
    pub limits: LogicLimits,
    pub cache_dir: Option<PathBuf>,
    pub dump_dir: PathBuf,
    pub theme: Theme,
    pub motion: MotionPolicy,
    pub gl: touchbar_gl::Backend,
}
impl SessionOptions {
    pub fn new(state_home: PathBuf, config: UserConfig) -> Self {
        let limits = LogicLimits {
            logic_hard_ms: config.logic.time_limit_ms,
            ..LogicLimits::default()
        };
        Self {
            config,
            limits,
            cache_dir: RuntimeConfig::default().cache_dir,
            dump_dir: state_home.join("touchbar/dumps"),
            theme: Theme::dark(),
            motion: MotionPolicy::Full,
            gl: touchbar_gl::Backend::Auto,
        }
    }
    pub fn core_config(&self) -> CoreConfig {
        let l = &self.limits;
        CoreConfig {
            logic_soft_ms: l.logic_soft_ms,
            logic_hard_ms: l.logic_hard_ms,
            max_update_bytes: l.max_update_bytes,
            max_requests_per_update: l.max_requests_per_update,
            max_subscriptions: l.max_subscriptions,
            max_text_bytes: l.max_text_bytes,
            max_records: l.max_records,
            max_persist_bytes: l.max_persist_bytes,
            ..CoreConfig::default()
        }
    }
}
#[derive(Clone, Debug)]
pub enum Event {
    Input(Input),
    Desktop(BackendEvent),
    Surface(SurfaceEvent),
    CommandError(String),
    Wake,
}
enum Work {
    Close(SurfaceId),
    Event(Event),
    Input(Input),
    Output(Output),
    Draw {
        surface: SurfaceId,
        generation: Generation,
        scene: Scene,
        animation: bool,
    },
    Present {
        surface: SurfaceId,
        frame: Arc<PresentedFrame>,
        started: Instant,
    },
}
struct SurfaceState {
    transport: Box<dyn Surface>,
    info: SurfaceInfo,
    pacer: FramePacer,
    generation: Option<Generation>,
    scene: Option<Scene>,
    animate: bool,
    visual_dirty: bool,
    queued: Option<(Generation, Scene)>,
    last: Option<Arc<PresentedFrame>>,
    gl_failures: u8,
    gl_retry: Option<Timestamp>,
    target: Option<touchbar_gl::ImportedTarget>,
    mode: Mode,
}
#[derive(Default, Clone, Debug)]
pub struct PumpStats {
    pub work: usize,
    pub inputs: usize,
    pub frames: usize,
    pub pending: bool,
}
#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Metrics {
    pub frame_ms: VecDeque<f64>,
    pub wasm_ms: VecDeque<f64>,
}
type CommandFuture =
    Pin<Box<dyn Future<Output = Result<(), touchbar_desktop::DesktopError>> + 'static>>;
struct KeyCommand {
    key: SystemKey,
    phase: KeyPhase,
    recovery: bool,
    future: Option<CommandFuture>,
}
trait PixelRenderer {
    fn render(
        &mut self,
        surface: SurfaceId,
        size: (u32, u32),
        list: &touchbar_ui::DisplayList,
    ) -> Result<Option<touchbar_gl::RenderedFrame>>;
    fn import_target(
        &mut self,
        _buffers: Vec<std::os::fd::OwnedFd>,
        _header: touchbar_protocol::SwapchainHeader,
    ) -> Result<touchbar_gl::ImportedTarget> {
        Err(anyhow!("renderer cannot import shared buffers"))
    }
    fn render_target(
        &mut self,
        _surface: SurfaceId,
        _target: &mut touchbar_gl::ImportedTarget,
        _index: u32,
        _list: &touchbar_ui::DisplayList,
        _full: bool,
    ) -> Result<Option<touchbar_gl::TargetFrame>> {
        Err(anyhow!("renderer cannot render shared buffers"))
    }
    fn remove_surface(&mut self, surface: SurfaceId);
}
impl PixelRenderer for touchbar_gl::Renderer {
    fn render(
        &mut self,
        surface: SurfaceId,
        size: (u32, u32),
        list: &touchbar_ui::DisplayList,
    ) -> Result<Option<touchbar_gl::RenderedFrame>> {
        self.render(
            surface,
            size,
            touchbar_gl::Orientation::Landscape,
            list,
            false,
        )
    }
    fn import_target(
        &mut self,
        buffers: Vec<std::os::fd::OwnedFd>,
        header: touchbar_protocol::SwapchainHeader,
    ) -> Result<touchbar_gl::ImportedTarget> {
        self.import_target(buffers, header)
    }
    fn render_target(
        &mut self,
        surface: SurfaceId,
        target: &mut touchbar_gl::ImportedTarget,
        index: u32,
        list: &touchbar_ui::DisplayList,
        full: bool,
    ) -> Result<Option<touchbar_gl::TargetFrame>> {
        self.render_target(surface, target, index, list, full)
    }
    fn remove_surface(&mut self, surface: SurfaceId) {
        self.remove_surface(surface);
    }
}
type RendererFactory = Box<dyn Fn(touchbar_gl::Backend) -> Result<Box<dyn PixelRenderer>>>;
enum SharedFrame {
    Deferred,
    Unchanged,
    Ready(touchbar_gl::TargetFrame),
}
struct FailedPersist {
    key: PersistKey,
    fields: StatePatch,
}
pub struct Session {
    core: Core,
    runtime: Runtime,
    recorder: Recorder,
    clock: Box<dyn Clock>,
    effects: Engine,
    desktop: Rc<dyn DesktopBackend>,
    monitor: BackendMonitor,
    loader: Loader,
    catalog: Catalog,
    documents: Documents,
    options: SessionOptions,
    renderer: SceneRenderer,
    painter: CpuPainter,
    gl: Option<Box<dyn PixelRenderer>>,
    gl_factory: RendererFactory,
    surfaces: BTreeMap<SurfaceId, SurfaceState>,
    work: VecDeque<Work>,
    timers: BTreeMap<TimerId, Timestamp>,
    commands: VecDeque<KeyCommand>,
    pressed_keys: BTreeSet<SystemKey>,
    diagnostics: Diagnostics,
    recent: VecDeque<Record>,
    log_context: BTreeMap<InstanceId, SetRef>,
    dumps: DumpStore,
    failed_persist: Vec<FailedPersist>,
    persist_retry: Option<Timestamp>,
    metrics: Metrics,
    pub(crate) touchbard_status: Option<crate::touchbard::Status>,
    pub(crate) last_calls: BTreeMap<InstanceId, Timestamp>,
    latest_load_issues: Vec<LoadIssue>,
    stopped: bool,
}
impl Session {
    pub fn new(
        options: SessionOptions,
        clock: impl Clock + 'static,
        persist: Box<dyn PersistStore>,
        desktop: Rc<dyn DesktopBackend>,
        effects: Engine,
    ) -> Result<Self> {
        Self::new_with_painter(
            options,
            clock,
            persist,
            desktop,
            effects,
            CpuPainter::new(touchbar_ui::ensure_default_font(
                cosmic_text::FontSystem::new(),
            )),
        )
    }
    /// An explicit painter makes font selection/locale reproducible for embeddings.
    pub fn new_with_painter(
        options: SessionOptions,
        clock: impl Clock + 'static,
        persist: Box<dyn PersistStore>,
        desktop: Rc<dyn DesktopBackend>,
        effects: Engine,
        painter: CpuPainter,
    ) -> Result<Self> {
        let config = options.core_config();
        config.validate().map_err(|e| anyhow!(e))?;
        ensure!(
            options.config.logic.time_limit_ms == options.limits.logic_hard_ms,
            "core/runtime hard limit must match startup config"
        );
        let runtime = Runtime::new(
            RuntimeConfig {
                limits: options.limits.clone(),
                cache_dir: options.cache_dir.clone(),
            },
            persist,
        )?;
        let core = Core::new(config);
        let recorder = Recorder::new(&core);
        let dumps = DumpStore::new(options.dump_dir.clone());
        let loader = Loader::new(options.config.clone());
        let mut this = Self {
            core,
            runtime,
            recorder,
            clock: Box::new(clock),
            effects,
            desktop,
            monitor: BackendMonitor::default(),
            loader,
            catalog: Catalog::default(),
            documents: Documents::default(),
            options,
            renderer: SceneRenderer::default(),
            painter,
            gl: None,
            gl_factory: Box::new(|backend| Ok(Box::new(touchbar_gl::Renderer::new(backend)?))),
            surfaces: BTreeMap::new(),
            work: VecDeque::new(),
            timers: BTreeMap::new(),
            commands: VecDeque::new(),
            pressed_keys: BTreeSet::new(),
            diagnostics: Diagnostics::default(),
            recent: VecDeque::new(),
            log_context: BTreeMap::new(),
            dumps,
            failed_persist: Vec::new(),
            persist_retry: None,
            metrics: Metrics::default(),
            touchbard_status: None,
            last_calls: BTreeMap::new(),
            latest_load_issues: Vec::new(),
            stopped: false,
        };
        this.handle_event(Event::Input(Input::Locale(locale_from(|name| {
            std::env::var(name).ok()
        }))))?;
        Ok(this)
    }
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }
    pub fn snapshot(&self) -> CoreSnapshot {
        self.core.snapshot()
    }
    pub fn dump(&self) -> ReplayDump {
        self.recorder.dump()
    }
    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }
    pub fn logs(&self) -> &VecDeque<Record> {
        &self.recent
    }
    pub fn compiler(&self) -> Compiler {
        self.runtime.compiler()
    }
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }
    pub fn config(&self) -> &UserConfig {
        &self.options.config
    }
    pub fn load_issues(&self) -> &[LoadIssue] {
        &self.latest_load_issues
    }
    pub fn has_pending(&self) -> bool {
        !self.work.is_empty()
    }
    fn enqueue(&mut self, work: Work) -> Result<()> {
        ensure!(
            self.work.len() < QUEUE_LIMIT,
            "session event queue exceeds {QUEUE_LIMIT}"
        );
        self.work.push_back(work);
        Ok(())
    }
    pub fn handle_event(&mut self, event: Event) -> Result<()> {
        ensure!(!self.stopped, "session is stopped");
        self.enqueue(Work::Event(event))
    }
    pub fn load(&mut self, prepared: PreparedLoad) -> LoadReport {
        // Admission may evict unused revisions. Resolve all earlier CreateInstance
        // outputs first so their live runtimes keep the revision pinned.
        if self.has_pending()
            && let Err(error) = self.pump()
        {
            return LoadReport {
                accepted: false,
                issues: vec![LoadIssue {
                    plugin: None,
                    message: error.to_string(),
                }],
            };
        }
        if self.has_pending() {
            return LoadReport {
                accepted: false,
                issues: vec![LoadIssue {
                    plugin: None,
                    message: "session is busy; retry catalog admission after pump".into(),
                }],
            };
        }
        let previously_loaded = self.loader.current().is_some();
        let report = self.loader.accept(prepared, &self.core, &mut self.runtime);
        self.latest_load_issues = report.issues.clone();
        if report.accepted {
            let accepted = self.loader.current().expect("accepted catalog");
            self.options.config = accepted.config.clone();
            if !previously_loaded || self.catalog != accepted.catalog {
                self.catalog = accepted.catalog.clone();
                self.documents = accepted.documents.clone();
                for surface in self.surfaces.values_mut() {
                    // Keep the last scene until Core replaces it. A catalog
                    // with no visual change need not emit another Render.
                    surface.queued = None;
                    surface.animate = false;
                    surface.pacer.stop_animation();
                }
                if let Err(error) = self.enqueue(Work::Input(Input::Catalog(self.catalog.clone())))
                {
                    return LoadReport {
                        accepted: false,
                        issues: vec![LoadIssue {
                            plugin: None,
                            message: error.to_string(),
                        }],
                    };
                }
            }
        }
        self.runtime_logs(None);
        for issue in &report.issues {
            self.error(None, &issue.message);
        }
        report
    }
    pub fn add_surface(&mut self, transport: Box<dyn Surface>) -> Result<()> {
        let info = transport.info();
        info.validate().map_err(|e| anyhow!(e))?;
        ensure!(
            self.surfaces.len() < 8
                && !self.surfaces.contains_key(&info.id)
                && !self.surfaces.values().any(|s| s.info.name == info.name),
            "duplicate surface or surface limit"
        );
        self.enqueue(Work::Input(Input::SurfaceAdded(info.clone())))?;
        self.surfaces.insert(
            info.id,
            SurfaceState {
                transport,
                info,
                pacer: FramePacer::default(),
                generation: None,
                scene: None,
                animate: false,
                visual_dirty: false,
                queued: None,
                last: None,
                gl_failures: 0,
                gl_retry: None,
                target: None,
                mode: Mode::Display,
            },
        );
        Ok(())
    }
    pub fn frame(&self, id: SurfaceId) -> Option<Arc<PresentedFrame>> {
        self.surfaces.get(&id).and_then(|s| s.last.clone())
    }
    pub fn next_deadline(&self) -> Option<Timestamp> {
        self.timers
            .values()
            .copied()
            .chain(
                self.surfaces
                    .values()
                    .filter_map(|s| s.gl_retry.or_else(|| s.pacer.deadline())),
            )
            .chain(self.diagnostics.deadline())
            .chain(self.persist_retry)
            .min()
    }
    pub async fn next_executor(&mut self) -> Event {
        futures_lite::future::or(async { Event::Input(self.effects.next().await) }, async {
            let (command, result) = futures_lite::future::poll_fn(|cx| {
                use std::task::Poll;
                let Some(command) = self.commands.front_mut() else {
                    return Poll::Pending;
                };
                if command.future.is_none() {
                    let work = self.desktop.command(DesktopCommand::Key {
                        key: command.key,
                        phase: command.phase,
                    });
                    command.future = Some(Box::pin(futures_lite::future::or(work, async {
                        async_io::Timer::after(Duration::from_secs(1)).await;
                        Err(touchbar_desktop::DesktopError::Invalid(
                            "key command timed out".into(),
                        ))
                    })));
                }
                if let Poll::Ready(result) = command
                    .future
                    .as_mut()
                    .expect("started key")
                    .as_mut()
                    .poll(cx)
                {
                    let command = self.commands.pop_front().expect("pending key");
                    return Poll::Ready((command, result));
                }
                Poll::Pending
            })
            .await;
            match result {
                Ok(()) => Event::Wake,
                Err(error) => {
                    // The failed call may have delivered a key before losing its reply.
                    // Remove stale pairs and make one bounded compensating release.
                    self.commands.retain(|queued| queued.key != command.key);
                    self.pressed_keys.remove(&command.key);
                    if !command.recovery {
                        self.commands.push_front(KeyCommand {
                            key: command.key,
                            phase: KeyPhase::Released,
                            recovery: true,
                            future: None,
                        });
                    }
                    Event::CommandError(format!(
                        "key command ({:?}, {:?}): {error}",
                        command.key, command.phase
                    ))
                }
            }
        })
        .await
    }
    pub fn pump(&mut self) -> Result<PumpStats> {
        let mut stats = PumpStats::default();
        let started = self.now();
        while stats.work < PUMP_LIMIT && (stats.work == 0 || self.now().elapsed_since(started) < 8)
        {
            if self.work.is_empty() {
                let repaint = self
                    .surfaces
                    .iter()
                    .filter(|(_, s)| {
                        s.visual_dirty
                            && !s.pacer.busy()
                            && s.gl_retry.is_none_or(|at| self.now() >= at)
                    })
                    .filter_map(|(id, s)| Some((*id, s.generation?, s.scene.clone()?)))
                    .collect::<Vec<_>>();
                for (surface, generation, scene) in repaint {
                    self.enqueue(Work::Draw {
                        surface,
                        generation,
                        scene,
                        animation: false,
                    })?;
                }
            }
            let Some(work) = self.work.pop_front() else {
                break;
            };
            stats.work += 1;
            match work {
                Work::Close(id) => {
                    if let Some(mut state) = self.surfaces.remove(&id) {
                        state.transport.close();
                    }
                    self.renderer.remove_surface(id);
                    if let Some(gl) = &mut self.gl {
                        gl.remove_surface(id);
                    }
                }
                Work::Event(event) => self.event(event)?,
                Work::Input(input) => {
                    let now = self.now();
                    self.before_input(&input)?;
                    let closed = match &input {
                        Input::SurfaceRemoved(id) | Input::Revoked { surface: id } => Some(*id),
                        _ => None,
                    };
                    let fault = matches!(
                        input,
                        Input::LogicFault { .. } | Input::InstanceFault { .. }
                    );
                    let outputs = self.recorder.handle(&mut self.core, now, input);
                    stats.inputs += 1;
                    for output in outputs {
                        self.enqueue(Work::Output(output))?;
                    }
                    if let Some(id) = closed {
                        self.enqueue(Work::Close(id))?;
                    }
                    if fault {
                        let dump = self.dump();
                        if let Err(error) = self.dumps.fault(now, &dump) {
                            self.error(None, &format!("dump: {error}"));
                        }
                    }
                }
                Work::Output(output) => self.output(output)?,
                Work::Draw {
                    surface,
                    generation,
                    scene,
                    animation,
                } => {
                    if self.draw(surface, generation, scene, animation)? {
                        stats.frames += 1;
                    }
                }
                Work::Present {
                    surface,
                    frame,
                    started,
                } => self.present(surface, frame, started)?,
            }
        }
        stats.pending = self.has_pending();
        Ok(stats)
    }
    fn before_input(&mut self, input: &Input) -> Result<()> {
        if let Input::Mode { surface, mode } = input
            && let Some(state) = self.surfaces.get_mut(surface)
        {
            state.mode = *mode;
        }
        match input {
            Input::SurfaceChanged(info) => {
                if let Some(state) = self.surfaces.get_mut(&info.id) {
                    info.validate().map_err(|e| anyhow!(e))?;
                    state.info = info.clone();
                }
            }
            Input::SurfaceRemoved(id)
            | Input::Revoked { surface: id }
            | Input::Mode {
                surface: id,
                mode: Mode::Native,
            } => {
                if let Some(state) = self.surfaces.get_mut(id) {
                    state.animate = false;
                    state.scene = None;
                    state.queued = None;
                    state.gl_retry = None;
                    state.visual_dirty = false;
                    state.pacer.cancel();
                    state.last = None;
                    state.target = None;
                }
            }
            Input::FrameDone {
                surface,
                generation,
            } => self.finished(*surface, *generation)?,
            Input::Fact { name, value } => {
                self.effects.fact(name.clone(), value.clone());
                if name.0 == "appearance.scheme" {
                    let theme = if matches!(value,Some(FactValue::Text(s))if s=="light") {
                        Theme::light()
                    } else {
                        Theme::dark()
                    };
                    if self.options.theme != theme {
                        self.options.theme = theme;
                        for state in self.surfaces.values_mut() {
                            state.visual_dirty = true;
                        }
                    }
                }
            }
            Input::LogicFault {
                instance, reason, ..
            }
            | Input::InstanceFault { instance, reason } => self.error(Some(*instance), reason),
            Input::LogicError {
                instance, message, ..
            } => self.error(Some(*instance), message),
            Input::SubscriptionLost {
                subscription,
                error,
            } => self.error(
                Some(subscription.instance),
                &format!("subscription: {error:?}"),
            ),
            _ => {}
        }
        Ok(())
    }
    fn event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Input(input) | Event::Surface(SurfaceEvent::Input(input)) => {
                self.enqueue(Work::Input(input))?
            }
            Event::Surface(SurfaceEvent::RetryPresent(id)) => {
                if let Some(frame) = self
                    .surfaces
                    .get(&id)
                    .filter(|s| s.pacer.busy())
                    .and_then(|s| s.last.clone())
                {
                    self.enqueue(Work::Present {
                        surface: id,
                        frame,
                        started: Instant::now(),
                    })?;
                }
            }
            Event::Surface(SurfaceEvent::Closed(id)) => {
                self.enqueue(Work::Input(Input::SurfaceRemoved(id)))?
            }
            Event::Surface(SurfaceEvent::Presented {
                surface,
                generation,
            }) => self.enqueue(Work::Input(Input::FrameDone {
                surface,
                generation,
            }))?,
            Event::Desktop(event) => match self.monitor.apply(event, self.now()) {
                Ok(delta) => {
                    if let Some(caps) = delta.capabilities {
                        self.enqueue(Work::Input(Input::DesktopCapabilities(
                            DesktopCapabilities {
                                key: caps.commands.contains(&CommandKind::Key),
                            },
                        )))?;
                    }
                    for (name, value) in delta.facts {
                        self.enqueue(Work::Input(Input::Fact { name, value }))?;
                    }
                }
                Err(error) => self.error(None, &error.to_string()),
            },
            Event::CommandError(error) => self.error(None, &error),
            Event::Wake => {
                let now = self.now();
                let due = self
                    .timers
                    .iter()
                    .filter(|(_, at)| **at <= now)
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                for id in due {
                    self.timers.remove(&id);
                    self.enqueue(Work::Input(Input::Timer { id }))?;
                }
                let frames = self
                    .surfaces
                    .iter()
                    .filter(|(_, s)| {
                        s.gl_retry.is_none()
                            && s.pacer.deadline().is_some_and(|at| at <= now)
                            && !s.pacer.busy()
                    })
                    .filter_map(|(id, s)| Some((*id, s.generation?, s.scene.clone()?)))
                    .collect::<Vec<_>>();
                for (surface, generation, scene) in frames {
                    self.enqueue(Work::Draw {
                        surface,
                        generation,
                        scene,
                        animation: true,
                    })?;
                }
                let records = self.diagnostics.tick(now);
                self.records(records);
                if self.persist_retry.is_some_and(|at| at <= now) {
                    self.retry_persist();
                }
            }
        }
        Ok(())
    }
    fn output(&mut self, output: Output) -> Result<()> {
        if self.stopped
            && matches!(
                output,
                Output::CreateInstance { .. }
                    | Output::CallLogic { .. }
                    | Output::StartEffect { .. }
                    | Output::Subscribe { .. }
                    | Output::Render { .. }
            )
        {
            return Ok(());
        }
        match output {
            Output::CreateInstance {
                instance,
                plugin,
                set,
                persist_key,
                ..
            } => {
                self.log_context.insert(instance, set.clone());
                let revision = self
                    .catalog
                    .plugins
                    .iter()
                    .find(|p| p.id == plugin)
                    .map(|p| p.revision.clone())
                    .unwrap_or_default();
                let input = self.runtime.create(
                    self.now(),
                    instance,
                    &plugin,
                    &revision,
                    &set,
                    &persist_key,
                );
                self.runtime_logs(Some(instance));
                self.enqueue(Work::Input(input))?;
            }
            Output::CallLogic {
                instance,
                call,
                invocation,
            } => {
                self.last_calls.insert(instance, self.now());
                let response = self.runtime.call(self.now(), instance, call, &invocation);
                Self::sample(&mut self.metrics.wasm_ms, response.duration);
                self.guest_logs(instance, response.logs);
                if let Some(input) = response.input {
                    self.enqueue(Work::Input(input))?;
                }
            }
            Output::DisposeInstance { instance } => {
                self.last_calls.remove(&instance);
                self.runtime.dispose(instance);
                self.runtime_logs(Some(instance));
                self.diagnostics.forget(instance);
                self.log_context.remove(&instance);
            }
            output @ (Output::StartEffect { .. }
            | Output::CancelEffect { .. }
            | Output::Subscribe { .. }
            | Output::Unsubscribe { .. }) => {
                self.effects.handle(output);
            }
            Output::SetTimer { id, at } => {
                self.timers.insert(id, at);
            }
            Output::ClearTimer { id } => {
                self.timers.remove(&id);
            }
            Output::Render {
                surface,
                generation,
                scene,
            } => self.enqueue(Work::Draw {
                surface,
                generation,
                scene,
                animation: false,
            })?,
            Output::Persist { key, fields, .. } => self.persist(key, fields),
            Output::SendKey {
                surface,
                key,
                phase,
            } => {
                if let Some(s) = self.surfaces.get_mut(&surface) {
                    let result = s.transport.send_key(key, phase);
                    if let Err(e) = result {
                        self.error(None, &e);
                    }
                }
            }
            Output::DesktopKey { key, phase, .. } => {
                if phase == KeyPhase::Pressed {
                    if self.pressed_keys.contains(&key) {
                        return Ok(());
                    }
                    // Every accepted press reserves a slot for its matching release.
                    if self.commands.len() + self.pressed_keys.len() + 2 > 1024 {
                        self.warning("too many pending desktop key commands; press rejected");
                        return Ok(());
                    }
                    self.pressed_keys.insert(key);
                } else if !self.pressed_keys.remove(&key) {
                    return Ok(());
                }
                self.commands.push_back(KeyCommand {
                    key,
                    phase,
                    recovery: false,
                    future: None,
                });
            }
            Output::WatchKeys { surface, set } => {
                if let Some(s) = self.surfaces.get_mut(&surface) {
                    let result = s.transport.watch_keys(&set);
                    if let Err(e) = result {
                        self.error(None, &e);
                    }
                }
            }
            Output::Backlight { surface, limit } => {
                if let Some(s) = self.surfaces.get_mut(&surface) {
                    let result = s.transport.backlight(limit);
                    if let Err(e) = result {
                        self.error(None, &e);
                    }
                }
            }
            Output::Log { level, event } => {
                if matches!(level, LogLevel::Warn | LogLevel::Error) {
                    let records = self.diagnostics.report(
                        self.now(),
                        None,
                        &event,
                        if level == LogLevel::Warn {
                            Level::Warn
                        } else {
                            Level::Error
                        },
                    );
                    self.records(records);
                } else {
                    self.records(vec![Record {
                        level: match level {
                            LogLevel::Debug => Level::Debug,
                            _ => Level::Info,
                        },
                        instance: None,
                        message: event,
                        suppressed: 0,
                    }]);
                }
            }
            Output::CaptureFinished { .. } => {}
        }
        Ok(())
    }
    fn draw(
        &mut self,
        id: SurfaceId,
        generation: Generation,
        scene: Scene,
        animation: bool,
    ) -> Result<bool> {
        let now = self.now();
        let Some(state) = self.surfaces.get_mut(&id) else {
            return Ok(false);
        };
        if state.mode == Mode::Native {
            return Ok(false);
        }
        if state.gl_retry.is_some_and(|at| now < at) {
            state.generation = Some(generation);
            state.scene = Some(scene);
            state.visual_dirty = true;
            self.enqueue(Work::Input(Input::FrameDone {
                surface: id,
                generation,
            }))?;
            return Ok(false);
        }
        if !state.pacer.begin(now, animation) {
            if !animation {
                state.queued = Some((generation, scene));
            }
            return Ok(false);
        }
        state.visual_dirty = false;
        state.generation = Some(generation);
        state.scene = Some(scene.clone());
        let info = state.info.clone();
        let start = Instant::now();
        let frame = self.renderer.frame(
            &info,
            &scene,
            &self.documents,
            self.options.theme,
            &mut self.painter,
            now,
            self.options.motion,
        );
        let frame = match frame {
            Ok(frame) => frame,
            Err(error) => {
                if let Some(state) = self.surfaces.get_mut(&id) {
                    state.animate = false;
                    state.scene = None;
                    state.gl_retry = None;
                    state.pacer.stop_animation();
                }
                self.error(None, &format!("frame: {error:#}"));
                self.enqueue(Work::Input(Input::FrameDone {
                    surface: id,
                    generation,
                }))?;
                return Ok(false);
            }
        };
        for (owner, error) in &frame.faults {
            self.enqueue(Work::Input(Input::InstanceFault {
                instance: *owner,
                reason: error.clone(),
            }))?;
        }
        for warning in &frame.warnings {
            let records = self
                .diagnostics
                .report(self.now(), None, warning, Level::Warn);
            self.records(records);
        }
        if let Some(state) = self.surfaces.get_mut(&id) {
            state.animate = frame.animate;
        }
        let layout = frame.layout.clone();
        let packet = Arc::new(PresentedFrame {
            generation,
            display_list: frame.display_list,
            layout: frame.layout,
            pixels: None,
            target: None,
        });
        // Present is a separate queue step: core applies Layout before any GL or
        // transport work. It cannot be re-entered from a surface callback.
        self.work.push_front(Work::Present {
            surface: id,
            frame: packet,
            started: start,
        });
        self.work.push_front(Work::Input(Input::Layout {
            surface: id,
            generation,
            hit_map: layout.hit_map,
            representations: layout.representations,
            scrubbers: layout.scrubbers,
            slots: layout.slots,
            presentation: layout.presentation,
        }));
        Ok(true)
    }
    fn present(
        &mut self,
        id: SurfaceId,
        mut frame: Arc<PresentedFrame>,
        started: Instant,
    ) -> Result<()> {
        let now = self.now();
        let Some(state) = self.surfaces.get_mut(&id) else {
            return Ok(());
        };
        if state.mode == Mode::Native {
            return Ok(());
        }
        if state.transport.uses_shared_target() {
            state.last = Some(frame.clone());
            let rendered: Result<SharedFrame> = (|| {
                let Some(slot) = state
                    .transport
                    .prepare_shared_target()
                    .map_err(|e| anyhow!(e))?
                else {
                    return Ok(SharedFrame::Deferred);
                };
                if self.gl.is_none() {
                    self.gl = Some((self.gl_factory)(self.options.gl.clone())?);
                }
                let gl = self.gl.as_mut().expect("initialized renderer");
                if let Some(buffers) = slot.buffers {
                    let target = gl.import_target(buffers, slot.header)?;
                    if let Some(warning) = target.warning() {
                        tracing::warn!(event = "touchbar-buffer-path", message = warning);
                    }
                    state.transport.target_imported(target.path());
                    state.target = Some(target);
                }
                let target = state
                    .target
                    .as_mut()
                    .ok_or_else(|| anyhow!("shared target missing"))?;
                let rendered =
                    gl.render_target(id, target, slot.index, &frame.display_list, slot.force_full)?;
                if let Some(rendered) = rendered {
                    Ok(SharedFrame::Ready(rendered))
                } else {
                    state
                        .transport
                        .frame_unchanged()
                        .map_err(anyhow::Error::msg)?;
                    Ok(SharedFrame::Unchanged)
                }
            })();
            match rendered {
                Ok(SharedFrame::Ready(rendered)) => {
                    Arc::make_mut(&mut frame).target = Some(rendered)
                }
                Ok(SharedFrame::Deferred) => return Ok(()),
                Ok(SharedFrame::Unchanged) => {
                    Self::sample(&mut self.metrics.frame_ms, started.elapsed());
                    self.enqueue(Work::Input(Input::FrameDone {
                        surface: id,
                        generation: frame.generation,
                    }))?;
                    return Ok(());
                }
                Err(error) => {
                    self.error(None, &format!("Touch Bar shared target: {error:#}"));
                    self.enqueue(Work::Input(Input::Revoked { surface: id }))?;
                    self.enqueue(Work::Input(Input::SurfaceRemoved(id)))?;
                    return Ok(());
                }
            }
        }
        if state.transport.needs_pixels() {
            let pixels = (|| {
                if self.gl.is_none() {
                    self.gl = Some((self.gl_factory)(self.options.gl.clone())?);
                }
                self.gl.as_mut().expect("initialized GL").render(
                    id,
                    (state.info.width_px, state.info.height_px),
                    &frame.display_list,
                )
            })();
            match pixels {
                Ok(pixels) => {
                    Arc::make_mut(&mut frame).pixels = pixels;
                    state.gl_failures = 0;
                    state.gl_retry = None;
                }
                Err(error) => {
                    self.gl = None;
                    state.gl_failures += 1;
                    let close = state.gl_failures >= 3;
                    state.gl_retry = (!close).then(|| now.after(100u64 << (state.gl_failures - 1)));
                    state.visual_dirty = !close;
                    // Stop animation deadlines while waiting for context recovery.
                    state.animate = false;
                    state.pacer.stop_animation();
                    let generation = frame.generation;
                    self.error(None, &format!("GL surface {}: {error:#}", id.0));
                    self.enqueue(Work::Input(Input::FrameDone {
                        surface: id,
                        generation,
                    }))?;
                    if close {
                        self.enqueue(Work::Input(Input::SurfaceRemoved(id)))?;
                    }
                    return Ok(());
                }
            }
        }
        state.last = Some(frame.clone());
        let generation = frame.generation;
        let result = state.transport.present(frame);
        Self::sample(&mut self.metrics.frame_ms, started.elapsed());
        match result {
            Ok(Presentation::Complete) => self.enqueue(Work::Input(Input::FrameDone {
                surface: id,
                generation,
            }))?,
            Ok(Presentation::Pending) => {}
            Err(error) => {
                self.error(None, &format!("surface: {error}"));
                self.enqueue(Work::Input(Input::SurfaceRemoved(id)))?;
            }
        }
        Ok(())
    }
    fn finished(&mut self, id: SurfaceId, generation: Generation) -> Result<()> {
        let Some(state) = self
            .surfaces
            .get_mut(&id)
            .filter(|s| s.generation == Some(generation))
        else {
            return Ok(());
        };
        state.pacer.finish(state.animate);
        if let Some((generation, scene)) = state.queued.take() {
            self.enqueue(Work::Draw {
                surface: id,
                generation,
                scene,
                animation: false,
            })?;
        }
        Ok(())
    }
    fn sample(samples: &mut VecDeque<f64>, duration: Duration) {
        if samples.len() == 512 {
            samples.pop_front();
        }
        samples.push_back(duration.as_secs_f64() * 1000.0);
    }
    fn error(&mut self, instance: Option<InstanceId>, text: &str) {
        let records = self.diagnostics.error(self.now(), instance, text);
        self.records(records);
    }
    pub(crate) fn warning(&mut self, text: &str) {
        let records = self.diagnostics.report(self.now(), None, text, Level::Warn);
        self.records(records);
    }
    fn records(&mut self, records: Vec<Record>) {
        for record in records {
            let instance = record.instance;
            let context = instance.and_then(|id| self.log_context.get(&id));
            macro_rules! emit {($level:expr)=>{{
                if tracing::enabled!($level) {
                    let plugin = context.map(|set| set.plugin.as_str()).unwrap_or("");
                    let set = context.map(ToString::to_string).unwrap_or_default();
                    tracing::event!($level,instance=?instance,plugin,set,suppressed=record.suppressed,message=%record.message);
                }
            }}}
            match record.level {
                Level::Trace => emit!(tracing::Level::TRACE),
                Level::Debug => emit!(tracing::Level::DEBUG),
                Level::Info => emit!(tracing::Level::INFO),
                Level::Warn => emit!(tracing::Level::WARN),
                Level::Error => emit!(tracing::Level::ERROR),
            }
            if self.recent.len() == 128 {
                self.recent.pop_front();
            }
            self.recent.push_back(record);
        }
    }
    fn runtime_logs(&mut self, instance: Option<InstanceId>) {
        let logs = self.runtime.take_logs();
        if let Some(instance) = instance {
            self.guest_logs(instance, logs);
        } else {
            for log in logs {
                let level = match log.level {
                    touchbar_logic_runtime::LogLevel::Trace => Level::Trace,
                    touchbar_logic_runtime::LogLevel::Debug => Level::Debug,
                    touchbar_logic_runtime::LogLevel::Info => Level::Info,
                    touchbar_logic_runtime::LogLevel::Warn => Level::Warn,
                    touchbar_logic_runtime::LogLevel::Error => Level::Error,
                };
                self.records(vec![Record {
                    level,
                    instance: None,
                    message: log.message,
                    suppressed: 0,
                }]);
            }
        }
    }
    fn guest_logs(&mut self, instance: InstanceId, logs: Vec<touchbar_logic_runtime::LogRecord>) {
        for log in logs {
            let mut records = self.diagnostics.guest(self.now(), instance, &log.message);
            for record in &mut records {
                if record.suppressed == 0 {
                    record.level = match log.level {
                        touchbar_logic_runtime::LogLevel::Trace => Level::Trace,
                        touchbar_logic_runtime::LogLevel::Debug => Level::Debug,
                        touchbar_logic_runtime::LogLevel::Info => Level::Info,
                        touchbar_logic_runtime::LogLevel::Warn => Level::Warn,
                        touchbar_logic_runtime::LogLevel::Error => Level::Error,
                    };
                }
            }
            self.records(records);
        }
    }
    fn merge_failed(&mut self, key: PersistKey, fields: StatePatch) {
        if let Some(old) = self.failed_persist.iter_mut().find(|p| p.key == key) {
            for (name, value) in fields {
                if let Some((_, previous)) = old.fields.iter_mut().find(|(k, _)| *k == name) {
                    *previous = value;
                } else {
                    old.fields.push((name, value));
                }
            }
        } else {
            self.failed_persist.push(FailedPersist { key, fields });
        }
    }
    fn persist(&mut self, key: PersistKey, fields: StatePatch) {
        self.merge_failed(key.clone(), fields);
        let index = self
            .failed_persist
            .iter()
            .position(|p| p.key == key)
            .expect("merged persist");
        let fields = self.failed_persist.remove(index).fields;
        match self.runtime.persist(&key, &fields) {
            Ok(()) => {
                self.failed_persist.retain(|p| p.key != key);
            }
            Err(error) => {
                self.error(None, &error.to_string());
                self.merge_failed(key, fields);
                self.persist_retry = Some(self.now().after(1000));
            }
        }
        self.runtime_logs(None);
    }
    fn retry_persist(&mut self) {
        self.persist_retry = None;
        for failed in std::mem::take(&mut self.failed_persist) {
            self.persist(failed.key, failed.fields);
        }
    }
    pub fn save_dump(&mut self) -> Result<PathBuf> {
        let dump = self.dump();
        self.dumps.manual(&dump).map_err(|e| anyhow!(e))
    }
    pub fn shutdown(&mut self) -> Result<()> {
        if self.stopped {
            self.retry_persist();
            ensure!(
                self.failed_persist.is_empty(),
                "persistent state could not be flushed"
            );
            return Ok(());
        }
        self.stopped = true;
        // Stop accepting guest/input work, preserving already accepted writes
        // and release commands whose core leases may already be gone.
        for work in std::mem::take(&mut self.work) {
            match work {
                Work::Output(Output::Persist { key, fields, .. }) => self.merge_failed(key, fields),
                Work::Output(
                    output @ (Output::SendKey {
                        phase: KeyPhase::Released,
                        ..
                    }
                    | Output::DesktopKey {
                        phase: KeyPhase::Released,
                        ..
                    }
                    | Output::DisposeInstance { .. }),
                ) => self.output(output)?,
                _ => {}
            }
        }
        for (key, fields) in self.core.pending_persist() {
            self.merge_failed(key, fields);
        }
        self.retry_persist();
        let ids = self.surfaces.keys().copied().collect::<Vec<_>>();
        for id in ids {
            self.enqueue(Work::Input(Input::SurfaceRemoved(id)))?;
        }
        while self.has_pending() {
            self.pump()?;
        }
        self.effects.shutdown();
        self.timers.clear();
        self.stopped = true;
        ensure!(
            self.failed_persist.is_empty(),
            "persistent state could not be flushed"
        );
        Ok(())
    }
    /// Drain serialized key releases and allow native spawn cancellation to
    /// escalate/reap, without blocking the asynchronous session thread.
    pub async fn finish_shutdown(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while (self.active_processes() > 0 || !self.commands.is_empty())
            && Instant::now() < deadline
        {
            let event = futures_lite::future::or(self.next_executor(), async {
                async_io::Timer::after(Duration::from_millis(10)).await;
                Event::Wake
            })
            .await;
            if let Event::CommandError(error) = event {
                self.error(None, &error);
            }
        }
        let incomplete = self.active_processes() > 0 || !self.commands.is_empty();
        self.commands.clear();
        ensure!(
            !incomplete,
            "session shutdown timed out while draining processes/keys"
        );
        Ok(())
    }
    pub fn active_processes(&self) -> usize {
        self.effects.active_processes()
    }
}
pub fn locale_from(mut env: impl FnMut(&str) -> Option<String>) -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|key| env(key).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "C".into())
}

#[cfg(test)]
mod log_context_tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Clone)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn logs_keep_create_context_until_dispose_even_without_core_instance() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let buffer = Buffer(bytes.clone());
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || buffer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let dir = tempfile::tempdir().unwrap();
            let desktop = Rc::new(touchbar_desktop::NullBackend::default());
            let (effects, _) = Engine::fake(desktop.clone());
            let mut session = Session::new(
                SessionOptions::new(dir.path().into(), UserConfig::default()),
                ManualClock::default(),
                Box::<touchbar_logic_runtime::MemoryStore>::default(),
                desktop,
                effects,
            )
            .unwrap();
            let instance = InstanceId {
                number: 42,
                generation: Generation(1),
            };
            session
                .output(Output::CreateInstance {
                    surface: SurfaceId(1),
                    instance,
                    plugin: "missing".parse().unwrap(),
                    set: "missing/volume".parse().unwrap(),
                    persist_key: PersistKey {
                        plugin: "missing".parse().unwrap(),
                        surface: "headless:test".parse().unwrap(),
                        scope: PersistScope::Layer("missing/volume".parse().unwrap()),
                    },
                })
                .unwrap();
            session.records(vec![Record {
                level: Level::Info,
                instance: Some(instance),
                message: "before-dispose".into(),
                suppressed: 0,
            }]);
            session
                .output(Output::DisposeInstance { instance })
                .unwrap();
            session.records(vec![Record {
                level: Level::Info,
                instance: Some(instance),
                message: "after-dispose".into(),
                suppressed: 0,
            }]);
        });
        let logs = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        let before = logs
            .lines()
            .find(|line| line.contains("before-dispose"))
            .unwrap();
        assert!(before.contains("plugin=\"missing\""), "{before}");
        assert!(before.contains("set=\"missing/volume\""), "{before}");
        let after = logs
            .lines()
            .find(|line| line.contains("after-dispose"))
            .unwrap();
        assert!(after.contains("plugin=\"\""), "{after}");
    }
}

#[cfg(test)]
mod local_failure_tests {
    use super::*;
    use std::sync::Mutex;
    fn session(desktop: Rc<dyn DesktopBackend>) -> (Session, ManualClock, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let clock = ManualClock::default();
        let (effects, _) = Engine::fake(desktop.clone());
        let session = Session::new(
            SessionOptions::new(dir.path().into(), UserConfig::default()),
            clock.clone(),
            Box::<touchbar_logic_runtime::MemoryStore>::default(),
            desktop,
            effects,
        )
        .unwrap();
        (session, clock, dir)
    }
    fn info(id: u64) -> SurfaceInfo {
        SurfaceInfo {
            id: SurfaceId(id),
            name: format!("headless:{id}").parse().unwrap(),
            kind: SurfaceKind::Headless,
            width_px: 400,
            height_px: 60,
            px_per_mm: 6.2,
            caps: SurfaceCaps::default(),
        }
    }
    struct Pixels(HeadlessSurface);
    impl Surface for Pixels {
        fn info(&self) -> SurfaceInfo {
            self.0.info()
        }
        fn needs_pixels(&self) -> bool {
            true
        }
        fn present(&mut self, frame: Arc<PresentedFrame>) -> Result<Presentation, String> {
            self.0.present(frame)
        }
    }
    #[test]
    fn gl_initialization_failure_is_bounded_and_closes_only_affected_surface() {
        let (mut session, clock, _dir) = session(Rc::new(touchbar_desktop::NullBackend::default()));
        session.options.gl =
            touchbar_gl::Backend::RenderNode("/definitely-missing-touchbar-render-node".into());
        let (pixels, _) = HeadlessSurface::new(info(1));
        let (other, handle) = HeadlessSurface::new(info(2));
        session.add_surface(Box::new(Pixels(pixels))).unwrap();
        session.add_surface(Box::new(other)).unwrap();
        session.pump().expect("GL failure must remain local");
        assert!(handle.frames() > 0);
        assert!(session.surfaces.contains_key(&SurfaceId(1)));
        for ms in [100, 300] {
            clock.set(Timestamp(ms)).unwrap();
            session.handle_event(Event::Wake).unwrap();
            session.pump().unwrap();
        }
        assert!(!session.surfaces.contains_key(&SurfaceId(1)));
        assert!(session.surfaces.contains_key(&SurfaceId(2)));
        session.dump().replay().unwrap();
    }
    #[test]
    fn native_mode_cancels_gl_recovery_deadline() {
        let (mut session, clock, _dir) = session(Rc::new(touchbar_desktop::NullBackend::default()));
        session.options.gl =
            touchbar_gl::Backend::RenderNode("/definitely-missing-touchbar-render-node".into());
        let (pixels, _) = HeadlessSurface::new(info(1));
        session.add_surface(Box::new(Pixels(pixels))).unwrap();
        session.pump().unwrap();
        session
            .handle_event(Event::Input(Input::Mode {
                surface: SurfaceId(1),
                mode: Mode::Native,
            }))
            .unwrap();
        session.pump().unwrap();
        clock.advance(100);
        session.handle_event(Event::Wake).unwrap();
        session.pump().unwrap();
        assert!(
            session.next_deadline().is_none_or(|at| at > clock.now()),
            "native surface retained expired retry"
        );
        assert!(!session.surfaces[&SurfaceId(1)].visual_dirty);
        session.dump().replay().unwrap();
    }
    struct ScriptRenderer {
        calls: Rc<std::cell::Cell<usize>>,
    }
    impl PixelRenderer for ScriptRenderer {
        fn render(
            &mut self,
            _: SurfaceId,
            (width, height): (u32, u32),
            _: &touchbar_ui::DisplayList,
        ) -> Result<Option<touchbar_gl::RenderedFrame>> {
            let call = self.calls.get() + 1;
            self.calls.set(call);
            if call == 2 {
                return Err(anyhow!("injected lost GL context"));
            }
            Ok(Some(touchbar_gl::RenderedFrame {
                width,
                height,
                pixels: vec![0; (width * height * 4) as usize],
                damage: vec![],
            }))
        }
        fn remove_surface(&mut self, _: SurfaceId) {}
    }
    #[test]
    fn failed_gl_frame_releases_generation_and_recreates_context_after_backoff() {
        let (mut session, clock, _dir) = session(Rc::new(touchbar_desktop::NullBackend::default()));
        let calls = Rc::new(std::cell::Cell::new(0));
        let created = Rc::new(std::cell::Cell::new(0));
        let (render_calls, contexts) = (calls.clone(), created.clone());
        session.gl_factory = Box::new(move |_| {
            contexts.set(contexts.get() + 1);
            Ok(Box::new(ScriptRenderer {
                calls: render_calls.clone(),
            }))
        });
        let (pixels, handle) = HeadlessSurface::new(info(1));
        session.add_surface(Box::new(Pixels(pixels))).unwrap();
        session.pump().unwrap();
        let mut resized = info(1);
        resized.width_px = 500;
        session
            .handle_event(Event::Input(Input::SurfaceChanged(resized)))
            .unwrap();
        session.pump().expect("failed render remains local");
        assert_eq!(handle.frames(), 1);
        assert!(!session.surfaces[&SurfaceId(1)].pacer.busy());
        assert_eq!(session.next_deadline(), Some(Timestamp(100)));
        clock.advance(99);
        session.handle_event(Event::Wake).unwrap();
        session.pump().unwrap();
        assert_eq!(calls.get(), 2);
        clock.advance(1);
        session.handle_event(Event::Wake).unwrap();
        session.pump().unwrap();
        assert_eq!(created.get(), 2);
        assert_eq!(handle.frames(), 2);
        assert_eq!(handle.frame().unwrap().pixels.as_ref().unwrap().width, 500);
        assert_eq!(session.surfaces[&SurfaceId(1)].gl_failures, 0);
        session.dump().replay().unwrap();
    }
    struct Keys {
        log: Arc<Mutex<Vec<(SystemKey, KeyPhase)>>>,
        stuck: bool,
    }
    impl DesktopBackend for Keys {
        fn name(&self) -> &'static str {
            "test-keys"
        }
        fn start(
            &mut self,
            _: async_channel::Sender<BackendEvent>,
        ) -> Result<(), touchbar_desktop::DesktopError> {
            Ok(())
        }
        fn command(&self, command: DesktopCommand) -> touchbar_desktop::CommandFuture {
            let DesktopCommand::Key { key, phase } = command else {
                panic!("unexpected command")
            };
            self.log.lock().unwrap().push((key, phase));
            if self.stuck && phase == KeyPhase::Pressed {
                Box::pin(std::future::pending())
            } else {
                Box::pin(std::future::ready(Ok(())))
            }
        }
    }
    fn key(session: &mut Session, phase: KeyPhase) {
        session
            .output(Output::DesktopKey {
                surface: SurfaceId(1),
                key: SystemKey::F1,
                phase,
            })
            .expect("backpressure must remain local");
    }
    #[test]
    fn full_key_queue_preserves_releases_and_does_not_dispatch_early() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (mut session, _, _dir) = session(Rc::new(Keys {
            log: log.clone(),
            stuck: false,
        }));
        for _ in 0..512 {
            key(&mut session, KeyPhase::Pressed);
            key(&mut session, KeyPhase::Released);
        }
        key(&mut session, KeyPhase::Pressed);
        key(&mut session, KeyPhase::Released);
        assert!(
            log.lock().unwrap().is_empty(),
            "commands must start serially on dispatch"
        );
        assert_eq!(session.commands.len(), 1024);
        futures_lite::future::block_on(async {
            for _ in 0..1024 {
                let event = session.next_executor().await;
                session.handle_event(event).unwrap();
                session.pump().unwrap();
            }
        });
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 1024);
        assert_eq!(log.last(), Some(&(SystemKey::F1, KeyPhase::Released)));
    }
    #[test]
    fn timed_out_key_does_not_block_release_or_keep_stale_presses() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let (mut session, _, _dir) = session(Rc::new(Keys {
            log: log.clone(),
            stuck: true,
        }));
        for _ in 0..3 {
            key(&mut session, KeyPhase::Pressed);
            key(&mut session, KeyPhase::Released);
        }
        futures_lite::future::block_on(async {
            let event = futures_lite::future::or(session.next_executor(), async {
                async_io::Timer::after(Duration::from_secs(3)).await;
                panic!("stuck key never timed out")
            })
            .await;
            assert!(matches!(event, Event::CommandError(_)));
            session.handle_event(event).unwrap();
            session.pump().unwrap();
            let event = session.next_executor().await;
            session.handle_event(event).unwrap();
            session.pump().unwrap();
        });
        assert!(session.commands.is_empty());
        assert_eq!(
            *log.lock().unwrap(),
            [
                (SystemKey::F1, KeyPhase::Pressed),
                (SystemKey::F1, KeyPhase::Released)
            ]
        );
    }
}
