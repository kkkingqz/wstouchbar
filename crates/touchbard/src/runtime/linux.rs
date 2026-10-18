//! Real drivers. All modesets, USB, sysfs writes and durable writes share one worker.
use super::{Drivers, marker::Recovery};
use crate::{
    Command, DisplayInfo, Event,
    backlight::{Autodim, Backlight, DriverSettings},
    config::{Config, ConfigStore},
    device::{
        self, Change, Device, DisplayNodes, ExclusiveDevice, FsMonitor, KernelMonitor, NativeNode,
        Roots, Tracker,
    },
    drm,
    input::TouchInput,
    keys::{InputBatch, KeyInputs, ModeWriter, Uinput},
    logind::{Monitor, Notice},
    machine::mode,
    worker::{Completion, SubmitError, Worker},
};
use async_io::Async;
use futures_lite::future::{or, pending};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs, io,
    os::fd::OwnedFd,
    path::PathBuf,
};
use touchbar_model::WatchKey;
use touchbar_protocol::Mode;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Scan,
    Usb,
    Open,
    Present(u64),
    Flip(u64),
    Release,
    Backlight,
    Recovery,
    Autodim,
    DriverMode,
}
impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Usb => "USB",
            Self::Open => "DRM open",
            Self::Present(_) => "DRM present",
            Self::Flip(_) => "DRM event",
            Self::Release => "DRM release",
            Self::Backlight => "backlight",
            Self::Recovery => "recovery",
            Self::Autodim => "autodim",
            Self::DriverMode => "Fn bridge",
        }
    }
    fn critical(self) -> bool {
        matches!(self, Self::Autodim | Self::Recovery | Self::DriverMode)
    }
    fn system(self) -> bool {
        matches!(
            self,
            Self::Scan | Self::Backlight | Self::Recovery | Self::Autodim | Self::DriverMode
        )
    }
}
#[derive(Clone)]
enum Operation {
    Drm(drm::Operation),
    Flips(Vec<drm::events::PageFlip>),
    Scan,
    Open {
        device: Device,
        nodes: DisplayNodes,
    },
    Backlight(u8),
    Recovery(bool),
    Autodim(bool),
    DriverMode {
        device: Device,
        path: PathBuf,
        value: u8,
    },
}
#[derive(Clone)]
struct Job {
    serial: u64,
    generation: u64,
    kind: Kind,
    operation: Operation,
}
enum Data {
    Done,
    Scanned(Option<device::Snapshot>, io::Result<(DriverSettings, bool)>),
    Opened(drm::Exports, TouchInput),
    Drm(drm::Reply),
    Autodim(bool),
    Frames(Vec<drm::Completion>),
}
struct Reply {
    serial: u64,
    generation: u64,
    kind: Kind,
    result: io::Result<Data>,
}
pub struct Linux {
    config: ConfigStore,
    log_filter: Option<super::logging::Filter>,
    admin: PathBuf,
    worker: Worker<Job, Reply>,
    tracker: Tracker,
    monitor: KernelMonitor,
    watcher: FsMonitor,
    logind: Monitor,
    keys: KeyInputs,
    uinput: Option<Uinput>,
    native: Option<NativeNode>,
    events: VecDeque<Event>,
    touch: Option<TouchInput>,
    exports: Option<(u64, drm::Exports)>,
    drm_events: Option<Async<OwnedFd>>,
    pending: VecDeque<Job>,
    flight: BTreeMap<u64, Job>,
    serial: u64,
    generation: u64,
    frame_connection: Option<u64>,
    wanted: BTreeSet<WatchKey>,
    reopen: Option<u64>,
    desired_level: Option<u8>,
    errors: BTreeMap<String, String>,
    cleanup_errors: BTreeMap<&'static str, String>,
    autodim: Option<bool>,
    recovery: bool,
    log_clock: std::time::Instant,
    repeats: super::logging::Repeats,
}
impl Linux {
    pub fn new(roots: Roots, admin: PathBuf) -> io::Result<Self> {
        let guard = ExclusiveDevice::acquire(&roots)?;
        let recovery = Recovery::new(&roots.run).read()?;
        let mut config = ConfigStore::default();
        if let Err(e) = config.reload(
            std::path::Path::new("/usr/share/touchbar/touchbard.toml"),
            &admin.join("touchbard.toml"),
        ) {
            eprintln!("touchbard: {e}");
        }
        let monitor = KernelMonitor::new()?;
        let tracker = Tracker::new(roots.clone());
        let watcher = FsMonitor::new(&Self::paths(&tracker, None))?;
        let keys = KeyInputs::open(&roots)?;
        let uinput = Uinput::open(&roots.dev.join("uinput"));
        let worker_roots = roots.clone();
        let mut execute = drm::executor(guard);
        let worker = Worker::new(move |job: Job| {
            let result = (|| match job.operation {
                Operation::Drm(op) => execute(op).map(Data::Drm),
                Operation::Flips(flips) => {
                    let mut frames = Vec::new();
                    for flip in flips {
                        if let drm::Reply::Completed(Some(c)) =
                            execute(drm::Operation::PageFlip(flip))?
                        {
                            frames.push(c);
                        }
                    }
                    Ok(Data::Frames(frames))
                }
                Operation::Scan => Ok(Data::Scanned(device::discover(&worker_roots)?, {
                    let a = Autodim::new(&worker_roots);
                    a.settings().and_then(|settings| Ok((settings, a.owned()?)))
                })),
                Operation::Backlight(level) => {
                    Backlight::open(&worker_roots)?.set_level(level)?;
                    Ok(Data::Done)
                }
                Operation::Recovery(value) => {
                    Recovery::new(&worker_roots.run).write(value)?;
                    Ok(Data::Done)
                }
                Operation::Autodim(own) => {
                    let control = Autodim::new(&worker_roots);
                    if own {
                        control.take_over()?;
                    } else {
                        control.restore()?;
                    }
                    Ok(Data::Autodim(control.settings()?.autodim.unwrap_or(false)))
                }
                Operation::DriverMode {
                    device,
                    path,
                    value,
                } => {
                    ModeWriter::open(&device, &path.canonicalize()?)?.write_mode(value)?;
                    Ok(Data::Done)
                }
                Operation::Open { device, nodes } => {
                    let dmi =
                        fs::read_to_string(worker_roots.sys.join("class/dmi/id/product_name")).ok();
                    let drm::Reply::Opened(exports) = execute(drm::Operation::Open {
                        card: nodes.card,
                        dmi_model: dmi.map(|s| s.trim().to_owned()),
                    })?
                    else {
                        unreachable!()
                    };
                    let touch = TouchInput::open(
                        &worker_roots,
                        &device,
                        &nodes.digitizer,
                        exports.header.logical_width,
                        exports.header.logical_height,
                    )?;
                    Ok(Data::Opened(exports, touch))
                }
            })();
            Reply {
                serial: job.serial,
                generation: job.generation,
                kind: job.kind,
                result,
            }
        })?;
        let mut this = Self {
            config,
            log_filter: None,
            admin,
            worker,
            tracker,
            monitor,
            watcher,
            logind: Monitor::new(),
            keys,
            uinput: None,
            native: None,
            events: VecDeque::new(),
            touch: None,
            exports: None,
            drm_events: None,
            pending: VecDeque::new(),
            flight: BTreeMap::new(),
            serial: 0,
            generation: 0,
            frame_connection: None,
            wanted: BTreeSet::new(),
            reopen: None,
            desired_level: None,
            errors: BTreeMap::new(),
            cleanup_errors: BTreeMap::new(),
            autodim: None,
            recovery,
            log_clock: std::time::Instant::now(),
            repeats: Default::default(),
        };
        match uinput {
            Ok(u) => this.uinput = Some(u),
            Err(e) => this.issue("uinput", e),
        }
        this.enqueue(Kind::Scan, Operation::Scan);
        this.pump();
        Ok(this)
    }
    pub fn set_log_filter(&mut self, filter: super::logging::Filter) {
        self.log_filter = Some(filter);
        for (source, error) in &self.errors {
            tracing::warn!(%source,%error,"driver initialization failed");
        }
    }
    pub fn config(&self) -> &Config {
        self.config.current()
    }
    pub fn recovery(&self) -> bool {
        self.recovery
    }
    pub fn idle(&self) -> bool {
        self.pending.is_empty() && self.flight.is_empty()
    }
    fn paths(tracker: &Tracker, native: Option<&NativeNode>) -> Vec<PathBuf> {
        let mut paths = tracker.watch_paths();
        paths.push(tracker.roots().sys.join("class/backlight"));
        paths.push(
            tracker
                .roots()
                .sys
                .join("module/hid_appletb_kbd/parameters"),
        );
        if let Some(n) = native {
            paths.push(n.path.clone());
        }
        paths
    }
    fn log_now(&self) -> touchbar_model::Timestamp {
        touchbar_model::Timestamp(
            u64::try_from(self.log_clock.elapsed().as_millis()).unwrap_or(u64::MAX),
        )
    }
    fn issue(&mut self, kind: &str, error: impl std::fmt::Display) {
        let message = error.to_string();
        for report in self.repeats.record(self.log_now(), kind, &message) {
            report.emit();
        }
        self.errors.insert(kind.into(), message);
    }
    fn enqueue(&mut self, kind: Kind, operation: Operation) {
        // Finite categories; keep only the latest desired value not yet submitted.
        self.pending.retain(|job| job.kind != kind);
        self.serial = self
            .serial
            .checked_add(1)
            .expect("device job serial exhausted");
        self.pending.push_back(Job {
            serial: self.serial,
            generation: self.generation,
            kind,
            operation,
        });
    }
    fn pump(&mut self) {
        while let Some(job) = self.pending.front().cloned() {
            let result = if job.kind.system() {
                self.worker.try_submit_committed(job.serial, job.clone())
            } else {
                self.worker.try_submit(job.generation, job.clone())
            };
            match result {
                Ok(()) => {
                    self.pending.pop_front();
                    self.flight.insert(job.serial, job);
                }
                Err(SubmitError::Full) => break,
                Err(e) => {
                    self.pending.pop_front();
                    self.issue("device queue", format!("{e:?}"));
                }
            }
        }
    }
    fn reset(&mut self, generation: u64) {
        self.generation = generation;
        let _ = self.worker.advance_generation(generation);
        self.pending.retain(|job| job.kind.system());
        self.flight.retain(|_, job| job.kind.system());
        self.touch = None;
        self.drm_events = None;
        self.exports = None;
        self.frame_connection = None;
        self.reopen = None;
    }
    fn scan(
        &mut self,
        snapshot: Option<device::Snapshot>,
        settings: io::Result<(DriverSettings, bool)>,
    ) -> io::Result<()> {
        match settings {
            Ok((settings, owned)) => {
                self.autodim = settings.autodim;
                if let Some(enabled) = settings.autodim {
                    self.events.push_back(Event::Autodim(enabled || owned));
                }
                self.events.push_back(Event::DriverTimeouts {
                    dim_seconds: settings.dim_seconds,
                    idle_seconds: settings.idle_seconds,
                });
            }
            Err(e) => self.issue("driver settings", e),
        }
        if self.reopen == Some(self.generation)
            && let Some(s) = &snapshot
            && s.config == Some(2)
            && let Some(nodes) = &s.display
        {
            self.enqueue(
                Kind::Open,
                Operation::Open {
                    device: s.device.clone(),
                    nodes: nodes.clone(),
                },
            );
            self.reopen = None;
        }
        for change in self.tracker.update(snapshot)? {
            match change {
                Change::Device(device) => {
                    self.events.push_back(Event::Device(device));
                }
                Change::NativeSafe { generation } => {
                    self.events.push_back(Event::NativeSafe { generation });
                }
                Change::NativeDriver { device, node } => {
                    let mode = node.as_ref().map(|n| n.mode);
                    self.native = node;
                    self.events.push_back(Event::Driver { device, mode });
                }
                Change::DisplayNodesReady {
                    generation,
                    device: _,
                    nodes,
                } => {
                    if generation == self.generation
                        && let Some((_, device)) = self.tracker.current()
                    {
                        self.enqueue(
                            Kind::Open,
                            Operation::Open {
                                device: device.clone(),
                                nodes,
                            },
                        );
                    }
                }
            }
        }
        self.watcher = FsMonitor::new(&Self::paths(&self.tracker, self.native.as_ref()))?;
        Ok(())
    }
    fn completed(&mut self, reply: Reply) {
        let job = self.flight.remove(&reply.serial);
        let result = (|| -> io::Result<()> {
            match reply.result? {
                Data::Scanned(snapshot, settings) => self.scan(snapshot, settings)?,
                Data::Opened(exports, touch) => {
                    let info = DisplayInfo {
                        header: exports.header,
                        caps: 1,
                        width_mm100: exports.physical_size.width_mm100,
                        height_mm100: exports.physical_size.height_mm100,
                    };
                    if let Some(warning) = &exports.physical_size.warning {
                        self.issue("physical size", warning);
                    }
                    self.drm_events = Some(Async::new(rustix::io::dup(&exports.events)?)?);
                    self.touch = Some(touch);
                    self.exports = Some((reply.generation, exports));
                    self.events.push_back(Event::DisplayReady {
                        generation: reply.generation,
                        info,
                    });
                }
                Data::Drm(drm::Reply::Done) => {
                    if reply.kind == Kind::Usb {
                        self.tracker.selected(reply.generation);
                        self.enqueue(Kind::Scan, Operation::Scan);
                        if let Some(level) = self.desired_level {
                            self.enqueue(Kind::Backlight, Operation::Backlight(level));
                        }
                    }
                }
                Data::Drm(
                    drm::Reply::Submitted(drm::Submission::Complete(completion))
                    | drm::Reply::Completed(Some(completion)),
                ) => self.presented(reply.generation, completion),
                Data::Frames(frames) => {
                    for c in frames {
                        self.presented(reply.generation, c);
                    }
                }
                Data::Autodim(enabled) => {
                    self.autodim = Some(enabled);
                }
                _ => {}
            }
            if reply.kind == Kind::Recovery
                && let Some(Job {
                    operation: Operation::Recovery(v),
                    ..
                }) = job
            {
                self.recovery = v;
            }
            Ok(())
        })();
        if let Err(e) = result {
            if reply.kind.critical() {
                self.cleanup_errors
                    .insert(reply.kind.label(), e.to_string());
            }
            self.issue(reply.kind.label(), &e);
            if !reply.kind.system() {
                self.tracker.failed(reply.generation);
                self.events.push_back(Event::TransitionFailed {
                    generation: reply.generation,
                    message: e.to_string(),
                });
            }
        } else if reply.kind.critical() {
            self.cleanup_errors.remove(reply.kind.label());
        }
        self.pump();
    }
    fn flips(&mut self, flips: Vec<drm::events::PageFlip>) {
        let Some(connection) = self.frame_connection else {
            return;
        };
        if let Some(job) = self
            .pending
            .iter_mut()
            .find(|j| j.kind == Kind::Flip(connection))
            && let Operation::Flips(queued) = &mut job.operation
        {
            if queued.len() + flips.len() <= 256 {
                queued.extend(flips);
            } else {
                self.events.push_back(Event::TransitionFailed {
                    generation: self.generation,
                    message: "DRM event queue exceeds bound".into(),
                });
            }
        } else {
            self.enqueue(Kind::Flip(connection), Operation::Flips(flips));
        }
    }
    fn presented(&mut self, generation: u64, completion: drm::Completion) {
        if let Some(connection) = self.frame_connection {
            let f = completion.frames.presented;
            self.events.push_back(Event::FramePresented {
                generation,
                connection,
                index: f.index,
                frame: f.frame,
                commit_us: completion.commit_us,
            });
            if let Some(f) = completion.frames.released {
                self.events.push_back(Event::FrameReleased {
                    generation,
                    connection,
                    index: f.index,
                    frame: f.frame,
                });
            }
        }
    }
    fn keys(&mut self, batch: InputBatch) -> Vec<Event> {
        for fault in batch.faults {
            self.issue(
                "evdev",
                format!("{}: {}", fault.path.display(), fault.error),
            );
        }
        let mut events = batch
            .changes
            .keys
            .into_iter()
            .map(Event::PhysicalKey)
            .collect::<Vec<_>>();
        if batch.changes.activity {
            events.push(Event::Activity);
        }
        if let Some(closed) = batch.changes.lid {
            events.push(Event::Lid(closed));
        }
        events
    }
}
impl Drivers for Linux {
    fn dispatch(&mut self, command: Command) -> Vec<Event> {
        match command {
            Command::Device(command) => {
                let generation = match command {
                    mode::Command::ToDisplay { generation, .. }
                    | mode::Command::ToNative { generation, .. }
                    | mode::Command::RefreshSwapchain { generation, .. }
                    | mode::Command::ReleaseHardware { generation } => generation,
                };
                self.reset(generation);
                match command {
                    mode::Command::ToDisplay { device, .. }
                    | mode::Command::ToNative { device, .. } => {
                        let mode = if matches!(command, mode::Command::ToDisplay { .. }) {
                            Mode::Display
                        } else {
                            Mode::Native
                        };
                        match self.tracker.begin_switch(generation, device, mode) {
                            Ok(plan) => {
                                self.enqueue(
                                    Kind::Usb,
                                    Operation::Drm(drm::Operation::Switch(plan)),
                                );
                            }
                            Err(e) => self.events.push_back(Event::TransitionFailed {
                                generation,
                                message: e.to_string(),
                            }),
                        }
                    }
                    mode::Command::RefreshSwapchain { .. } => {
                        // A new owner gets new allocations without a USB round trip.
                        self.reopen = Some(generation);
                        self.enqueue(Kind::Release, Operation::Drm(drm::Operation::Release));
                        self.enqueue(Kind::Scan, Operation::Scan);
                    }
                    mode::Command::ReleaseHardware { .. } => {
                        self.enqueue(Kind::Release, Operation::Drm(drm::Operation::Release))
                    }
                }
            }
            Command::Present {
                connection,
                index,
                frame,
                damage,
                ..
            } => {
                self.frame_connection = Some(connection);
                self.enqueue(
                    Kind::Present(connection),
                    Operation::Drm(drm::Operation::Present {
                        frame: drm::Frame { index, frame },
                        damage,
                        mode: drm::CommitMode::Nonblocking,
                    }),
                );
            }
            Command::AcquireInhibitor { generation } => self.logind.acquire_inhibitor(generation),
            Command::ReleaseInhibitor { generation } => self.logind.release_inhibitor(generation),
            Command::OwnAutodim => self.enqueue(Kind::Autodim, Operation::Autodim(true)),
            Command::RestoreAutodim => self.enqueue(Kind::Autodim, Operation::Autodim(false)),
            Command::WriteRecovery(value) => {
                self.enqueue(Kind::Recovery, Operation::Recovery(value))
            }
            Command::SetBacklight(level) => {
                self.desired_level = Some(level);
                self.enqueue(Kind::Backlight, Operation::Backlight(level));
            }
            Command::WriteDriverMode(value) => {
                if let Some((_, device)) = self.tracker.current()
                    && let Some(node) = &self.native
                {
                    self.enqueue(
                        Kind::DriverMode,
                        Operation::DriverMode {
                            device: device.clone(),
                            path: node.path.clone(),
                            value,
                        },
                    );
                }
            }
            Command::Uinput { key, phase } => {
                if let Some(uinput) = &mut self.uinput
                    && let Err(e) = uinput.send(key, phase)
                {
                    self.issue("uinput", e);
                    self.uinput = None;
                }
            }
            Command::Exit => {
                if let Some(u) = &mut self.uinput {
                    let _ = u.release_all();
                }
            }
            _ => {}
        }
        self.pump();
        self.events.drain(..).collect()
    }
    fn watch_keys(&mut self, wanted: BTreeSet<WatchKey>) -> Vec<Event> {
        if self.wanted == wanted {
            return Vec::new();
        }
        self.wanted = wanted.clone();
        self.keys.set_wanted(wanted);
        let batch = self.keys.refresh();
        self.keys(batch)
    }
    fn buffers(&self, generation: u64) -> Option<&[OwnedFd]> {
        self.exports
            .as_ref()
            .filter(|(g, _)| *g == generation)
            .map(|(_, e)| e.buffers.as_slice())
    }
    fn reload(&mut self) -> io::Result<Config> {
        self.config
            .reload(
                std::path::Path::new("/usr/share/touchbar/touchbard.toml"),
                &self.admin.join("touchbard.toml"),
            )
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
        if let Some(handle) = &self.log_filter
            && let Err(e) = handle.reload(tracing_subscriber::EnvFilter::new(
                &self.config.current().log,
            ))
        {
            self.issue("log filter", e);
        }
        Ok(self.config.current().clone())
    }
    fn status(&self) -> serde_json::Value {
        serde_json::json!({"autodim":self.autodim,"errors":self.errors,"pending":self.pending.len(),"in_flight":self.flight.len(),"uinput":self.uinput.is_some(),"recovery_persisted":self.recovery})
    }
    async fn finish(&mut self, deadline: std::time::Instant) -> io::Result<()> {
        self.pump();
        while self
            .pending
            .iter()
            .chain(self.flight.values())
            .any(|job| matches!(job.kind, Kind::Autodim | Kind::Recovery | Kind::DriverMode))
        {
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "system cleanup exceeded shutdown deadline",
                ));
            }
            or(self.next(), async {
                async_io::Timer::at(deadline).await;
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "system cleanup exceeded shutdown deadline",
                ))
            })
            .await?;
        }
        if self.cleanup_errors.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "system cleanup failed: {:?}",
                self.cleanup_errors
            )))
        }
    }
    async fn next(&mut self) -> io::Result<Vec<Event>> {
        self.pump();
        if !self.events.is_empty() {
            return Ok(self.events.drain(..).collect());
        }
        let log_deadline = self
            .repeats
            .deadline()
            .map(|t| self.log_clock + std::time::Duration::from_millis(t.0));
        enum Ready {
            LogSummary,
            Worker(Result<Completion<Reply>, crate::worker::WorkerClosed>),
            Kernel(io::Result<()>),
            Fs(io::Result<()>),
            Logind(io::Result<Notice>),
            Keys(io::Result<InputBatch>),
            Touch(crate::input::Batch),
            Drm(io::Result<Vec<drm::events::PageFlip>>),
        }
        let ready = or(
            async {
                if let Some(at) = log_deadline {
                    async_io::Timer::at(at).await;
                } else {
                    pending::<()>().await;
                }
                Ready::LogSummary
            },
            or(
                async { Ready::Worker(self.worker.next().await) },
                or(
                    async { Ready::Kernel(self.monitor.next().await.map(|_| ())) },
                    or(
                        async { Ready::Fs(self.watcher.next().await) },
                        or(
                            async { Ready::Logind(self.logind.next().await) },
                            or(
                                async { Ready::Keys(self.keys.next().await) },
                                or(
                                    async {
                                        match &mut self.touch {
                                            Some(t) => Ready::Touch(t.next().await),
                                            None => pending().await,
                                        }
                                    },
                                    async {
                                        let Some(fd) = &self.drm_events else {
                                            return pending().await;
                                        };
                                        Ready::Drm(
                                            fd.read_with(|fd| {
                                                let mut bytes = [0u8; 4096];
                                                let n = rustix::io::read(fd, &mut bytes)?;
                                                if n == 0 {
                                                    return Err(io::ErrorKind::UnexpectedEof.into());
                                                }
                                                drm::events::decode_events(&bytes[..n])
                                            })
                                            .await,
                                        )
                                    },
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        )
        .await;
        match ready {
            Ready::LogSummary => {
                for r in self.repeats.take_due(self.log_now()) {
                    r.emit();
                }
            }
            Ready::Worker(Ok(reply)) => self.completed(reply.value),
            Ready::Worker(Err(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "device worker stopped",
                ));
            }
            Ready::Kernel(Ok(())) | Ready::Fs(Ok(())) => self.enqueue(Kind::Scan, Operation::Scan),
            Ready::Kernel(Err(e)) | Ready::Fs(Err(e)) => {
                self.issue("device monitor", e);
                return Err(io::Error::other("device monitor unavailable"));
            }
            Ready::Logind(Ok(Notice::Fact(event))) => self.events.push_back(event),
            Ready::Logind(Ok(Notice::Unavailable(error))) => self.issue("logind", error),
            Ready::Logind(Err(e)) => self.issue("logind", e),
            Ready::Keys(Ok(batch)) => {
                let events = self.keys(batch);
                self.events.extend(events);
            }
            Ready::Keys(Err(e)) => {
                self.issue("evdev", e);
                return Err(io::Error::other("evdev collection unavailable"));
            }
            Ready::Touch(batch) => {
                self.events
                    .extend(batch.events.into_iter().map(|s| Event::Touch {
                        tracking: s.tracking,
                        phase: s.phase,
                        x: s.x,
                        y: s.y,
                    }));
                if let Some(e) = batch.fault {
                    self.touch = None;
                    self.events.push_back(Event::TransitionFailed {
                        generation: self.generation,
                        message: e.to_string(),
                    });
                }
            }
            Ready::Drm(Ok(flips)) => self.flips(flips),
            Ready::Drm(Err(e)) => {
                self.drm_events = None;
                self.events.push_back(Event::TransitionFailed {
                    generation: self.generation,
                    message: e.to_string(),
                });
            }
        }
        self.pump();
        Ok(self.events.drain(..).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Linux, device::Snapshot) {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots {
            sys: dir.path().join("sys"),
            dev: dir.path().join("dev"),
            run: dir.path().join("run"),
        };
        let usb = roots.sys.join("devices/usb/1-1");
        fs::create_dir_all(&usb).unwrap();
        fs::create_dir_all(&roots.dev).unwrap();
        fs::create_dir_all(roots.sys.join("bus/usb/devices")).unwrap();
        for (name, value) in [
            ("idVendor", "05ac\n"),
            ("idProduct", "8302\n"),
            ("bConfigurationValue", "2\n"),
        ] {
            fs::write(usb.join(name), value).unwrap();
        }
        std::os::unix::fs::symlink(&usb, roots.sys.join("bus/usb/devices/1-1")).unwrap();
        let mut snapshot = device::discover(&roots).unwrap().unwrap();
        snapshot.display = Some(DisplayNodes {
            card: roots.dev.join("dri/card0"),
            digitizer: roots.dev.join("input/event0"),
        });
        let linux = Linux::new(roots, dir.path().join("admin")).unwrap();
        (dir, linux, snapshot)
    }
    #[test]
    fn refresh_reopens_nodes_even_after_tracker_already_reported_readiness() {
        let (_dir, mut linux, snapshot) = fixture();
        linux.tracker.update(Some(snapshot.clone())).unwrap();
        let id = linux.tracker.current().unwrap().0;
        linux.tracker.begin_switch(1, id, Mode::Display).unwrap();
        linux.tracker.selected(1);
        assert!(
            linux
                .tracker
                .update(Some(snapshot.clone()))
                .unwrap()
                .iter()
                .any(|c| matches!(c, Change::DisplayNodesReady { .. }))
        );
        linux.reset(2);
        linux.reopen = Some(2);
        linux
            .scan(
                Some(snapshot),
                Ok((
                    DriverSettings {
                        autodim: None,
                        dim_seconds: None,
                        idle_seconds: None,
                    },
                    false,
                )),
            )
            .unwrap();
        assert_eq!(
            linux
                .pending
                .iter()
                .filter(|j| matches!(j.operation, Operation::Open { .. }) && j.generation == 2)
                .count(),
            1
        );
        assert_eq!(linux.reopen, None);
    }
    #[test]
    fn drm_batches_preserve_all_flips_and_mode_change_discards_them() {
        let (_dir, mut linux, _) = fixture();
        linux.frame_connection = Some(3);
        let flip = |n| drm::events::PageFlip {
            crtc: 7,
            sequence: n,
            timestamp_us: u64::from(n),
        };
        linux.flips(vec![flip(1)]);
        linux.flips(vec![flip(2), flip(3)]);
        let job = linux
            .pending
            .iter()
            .find(|j| j.kind == Kind::Flip(3))
            .unwrap();
        assert!(matches!(&job.operation,Operation::Flips(v) if v==&[flip(1),flip(2),flip(3)]));
        linux.reset(1);
        assert!(
            !linux
                .pending
                .iter()
                .any(|j| matches!(j.kind, Kind::Flip(_)))
        );
    }
}
