use crate::{
    clock::*,
    diagnostics::{Diagnostics, DumpStore, Level, Record},
    surface::*,
};
use anyhow::{Result, anyhow, ensure};
use std::{
    collections::{BTreeMap, VecDeque},
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
    gl: Option<touchbar_gl::Renderer>,
    surfaces: BTreeMap<SurfaceId, SurfaceState>,
    work: VecDeque<Work>,
    timers: BTreeMap<TimerId, Timestamp>,
    commands: VecDeque<CommandFuture>,
    diagnostics: Diagnostics,
    recent: VecDeque<Record>,
    dumps: DumpStore,
    failed_persist: Vec<FailedPersist>,
    persist_retry: Option<Timestamp>,
    metrics: Metrics,
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
            surfaces: BTreeMap::new(),
            work: VecDeque::new(),
            timers: BTreeMap::new(),
            commands: VecDeque::new(),
            diagnostics: Diagnostics::default(),
            recent: VecDeque::new(),
            dumps,
            failed_persist: Vec::new(),
            persist_retry: None,
            metrics: Metrics::default(),
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
            .chain(self.surfaces.values().filter_map(|s| s.pacer.deadline()))
            .chain(self.diagnostics.deadline())
            .chain(self.persist_retry)
            .min()
    }
    pub async fn next_executor(&mut self) -> Event {
        futures_lite::future::or(async { Event::Input(self.effects.next().await) }, async {
            futures_lite::future::poll_fn(|cx| {
                use std::task::Poll;
                if let Some(command) = self.commands.front_mut()
                    && let Poll::Ready(result) = command.as_mut().poll(cx)
                {
                    self.commands.pop_front();
                    return Poll::Ready(result);
                }
                Poll::Pending
            })
            .await
            .map_or_else(
                |e| Event::CommandError(format!("key command: {e}")),
                |()| Event::Wake,
            )
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
                    .filter(|(_, s)| s.visual_dirty && !s.pacer.busy())
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
                    state.pacer.stop_animation();
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
                        s.pacer.deadline().is_some_and(|at| at <= now) && !s.pacer.busy()
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
                ensure!(
                    self.commands.len() < 1024,
                    "too many pending desktop key commands"
                );
                self.commands
                    .push_back(self.desktop.command(DesktopCommand::Key { key, phase }));
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
        let Some(state) = self.surfaces.get_mut(&id) else {
            return Ok(());
        };
        if state.transport.needs_pixels() {
            let gl = match &mut self.gl {
                Some(gl) => gl,
                None => {
                    self.gl = Some(touchbar_gl::Renderer::new(self.options.gl.clone())?);
                    self.gl.as_mut().expect("initialized GL")
                }
            };
            let pixels = gl.render(
                id,
                (state.info.width_px, state.info.height_px),
                touchbar_gl::Orientation::Landscape,
                &frame.display_list,
                false,
            )?;
            Arc::make_mut(&mut frame).pixels = pixels;
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
    fn records(&mut self, records: Vec<Record>) {
        for record in records {
            let instance = record.instance;
            let context = instance.and_then(|id| {
                self.core
                    .snapshot()
                    .instances
                    .into_iter()
                    .find(|i| i.id == id)
            });
            let plugin = context.as_ref().map(|i| i.set.plugin.as_str());
            let set = context.as_ref().map(|i| i.set.to_string());
            macro_rules! emit {($level:expr)=>{{tracing::event!($level,instance=?instance,plugin=plugin.unwrap_or(""),set=set.as_deref().unwrap_or(""),suppressed=record.suppressed,message=%record.message);}}}
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
