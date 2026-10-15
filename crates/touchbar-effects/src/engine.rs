use crate::{
    dbus,
    fake::{self, FakeControl},
    spawn::{Process, ProcessEvent, SpawnPool},
};
use futures_lite::{Stream, stream};
use std::{
    collections::{BTreeSet, VecDeque},
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex, MutexGuard, Weak},
    task::Poll,
    time::{Duration, Instant},
};
use touchbar_audio::{AudioBackend, AudioCommand};
use touchbar_core::{Input, Output};
use touchbar_desktop::{DesktopBackend, DesktopCommand, DesktopError};
use touchbar_model::*;
type Job = Pin<Box<dyn Future<Output = EffectOutcome>>>;
type SourceStream = Pin<Box<dyn Stream<Item = Result<SubscriptionData, EffectError>>>>;
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn failed(e: impl std::fmt::Display) -> EffectError {
    EffectError::Failed(e.to_string())
}
pub struct Config {
    pub spawn: SpawnPool,
    pub dbus: dbus::Client,
}
impl Config {
    pub fn new(helper: std::path::PathBuf) -> Self {
        Self {
            spawn: SpawnPool::new(helper, 4, 64),
            dbus: dbus::Client::default(),
        }
    }
}
#[derive(Clone)]
struct Resources {
    desktop: Rc<dyn DesktopBackend>,
    audio: Arc<dyn AudioBackend>,
    native: Option<Arc<Config>>,
    fake: Option<FakeControl>,
}
struct Running {
    request: RequestId,
    future: Job,
}
struct Source {
    kind: SubscriptionKind,
    ids: BTreeSet<SubscriptionId>,
    owners: Arc<Mutex<BTreeSet<InstanceId>>>,
    process: Arc<Mutex<Option<Weak<Process>>>>,
    stream: SourceStream,
    last: Option<SubscriptionData>,
}
impl Source {
    fn update_owners(&self) {
        let owners = self.ids.iter().map(|id| id.instance).collect();
        *lock(&self.owners) = owners;
        let owners = lock(&self.owners).clone();
        if let Some(process) = lock(&self.process).as_ref().and_then(Weak::upgrade) {
            process.set_owners(owners);
        }
    }
}
pub struct Engine {
    resources: Resources,
    jobs: Vec<Running>,
    sources: Vec<Source>,
    pending: VecDeque<Input>,
    facts: Facts,
    source_cursor: usize,
    prefer_sources: bool,
}
impl Engine {
    /// Session termination cancels all owned sources and jobs; process workers
    /// retain quota until the supervised groups have actually been reaped.
    pub fn shutdown(&mut self) {
        self.jobs.clear();
        self.sources.clear();
        self.pending.clear();
    }
    pub fn active_processes(&self) -> usize {
        self.resources
            .native
            .as_ref()
            .map_or(0, |c| c.spawn.active())
    }

    pub fn native(
        config: Config,
        desktop: Rc<dyn DesktopBackend>,
        audio: Arc<dyn AudioBackend>,
    ) -> Self {
        Self::new(Resources {
            desktop,
            audio,
            native: Some(Arc::new(config)),
            fake: None,
        })
    }
    pub fn fake(desktop: Rc<dyn DesktopBackend>) -> (Self, FakeControl) {
        let control = FakeControl::default();
        (
            Self::new(Resources {
                desktop,
                audio: Arc::new(touchbar_audio::UnavailableAudio),
                native: None,
                fake: Some(control.clone()),
            }),
            control,
        )
    }
    fn new(resources: Resources) -> Self {
        Self {
            resources,
            jobs: vec![],
            sources: vec![],
            pending: VecDeque::new(),
            facts: Facts::new(),
            source_cursor: 0,
            prefer_sources: false,
        }
    }
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }
    pub fn active_requests(&self) -> usize {
        self.jobs.len()
    }
    pub fn fact(&mut self, name: FactName, value: Option<FactValue>) {
        if let Some(v) = &value {
            self.facts.insert(name.clone(), v.clone());
        } else {
            self.facts.remove(&name);
        }
        for source in &self.sources {
            if source.kind == (SubscriptionKind::DesktopFact { name: name.clone() }) {
                for subscription in &source.ids {
                    self.pending.push_back(Input::SubscriptionData {
                        subscription: *subscription,
                        data: SubscriptionData::Fact(value.clone()),
                    });
                }
            }
        }
    }
    pub fn handle(&mut self, output: Output) -> bool {
        match output {
            Output::StartEffect {
                request,
                effect,
                policy,
            } => {
                if self.jobs.iter().any(|j| j.request == request) {
                    return true;
                }
                if let Some(fake) = &self.resources.fake {
                    fake.start(request, &effect);
                }
                let resources = self.resources.clone();
                let future = Box::pin(async move {
                    let work = execute(resources, request, effect);
                    if let Some(ms) = policy.timeout_ms {
                        futures_lite::future::race(work, async move {
                            async_io::Timer::after(Duration::from_millis(ms)).await;
                            Err(EffectError::Timeout)
                        })
                        .await
                    } else {
                        work.await
                    }
                });
                self.jobs.push(Running { request, future });
            }
            Output::CancelEffect { request } => {
                self.jobs.retain(|j| j.request != request);
                self.pending
                    .retain(|i| !matches!(i,Input::EffectDone{request:r,..}if *r==request));
            }
            Output::Subscribe { subscription, kind } => {
                if self.sources.iter().any(|s| s.ids.contains(&subscription)) {
                    return true;
                }
                if let Some(source) = self.sources.iter_mut().find(|s| s.kind == kind) {
                    source.ids.insert(subscription);
                    source.update_owners();
                    if let SubscriptionKind::DesktopFact { name } = &kind {
                        self.pending.push_back(Input::SubscriptionData {
                            subscription,
                            data: SubscriptionData::Fact(self.facts.get(name).cloned()),
                        });
                    } else if let Some(data) = &source.last {
                        self.pending.push_back(Input::SubscriptionData {
                            subscription,
                            data: data.clone(),
                        });
                    }
                    return true;
                }
                let owners = Arc::new(Mutex::new([subscription.instance].into()));
                let process = Arc::new(Mutex::new(None));
                let stream = make_source(
                    self.resources.clone(),
                    kind.clone(),
                    owners.clone(),
                    process.clone(),
                );
                self.sources.push(Source {
                    kind: kind.clone(),
                    ids: [subscription].into(),
                    owners,
                    process,
                    stream,
                    last: None,
                });
                if let SubscriptionKind::DesktopFact { name } = kind {
                    self.pending.push_back(Input::SubscriptionData {
                        subscription,
                        data: SubscriptionData::Fact(self.facts.get(&name).cloned()),
                    });
                }
            }
            Output::Unsubscribe { subscription } => {
                for s in &mut self.sources {
                    if s.ids.remove(&subscription) {
                        s.update_owners();
                    }
                }
                self.sources.retain(|s| !s.ids.is_empty());
                self.pending.retain(|i|!matches!(i,Input::SubscriptionData{subscription:s,..}|Input::SubscriptionLost{subscription:s,..}if *s==subscription));
            }
            _ => return false,
        }
        true
    }
    pub async fn next(&mut self) -> Input {
        std::future::poll_fn(|cx| {
            if let Some(input) = self.pending.pop_front() {
                return Poll::Ready(input);
            }
            if self.prefer_sources
                && let Poll::Ready(input) = self.poll_source(cx)
            {
                return Poll::Ready(input);
            }
            if let Poll::Ready(input) = self.poll_job(cx) {
                return Poll::Ready(input);
            }
            self.poll_source(cx)
        })
        .await
    }
    fn poll_job(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Input> {
        for i in 0..self.jobs.len() {
            if let Poll::Ready(result) = self.jobs[i].future.as_mut().poll(cx) {
                let job = self.jobs.remove(i);
                self.prefer_sources = true;
                return Poll::Ready(Input::EffectDone {
                    request: job.request,
                    result,
                });
            }
        }
        Poll::Pending
    }
    fn poll_source(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Input> {
        let len = self.sources.len();
        for offset in 0..len {
            let i = (self.source_cursor + offset) % len;
            if let Poll::Ready(item) = self.sources[i].stream.as_mut().poll_next(cx) {
                let ids = self.sources[i].ids.clone();
                match item {
                    Some(Ok(data)) => {
                        if matches!(
                            self.sources[i].kind,
                            SubscriptionKind::AudioWatch { .. }
                                | SubscriptionKind::MediaWatch
                                | SubscriptionKind::DbusProperty(_)
                        ) {
                            self.sources[i].last = Some(data.clone());
                        }
                        for id in ids {
                            self.pending.push_back(Input::SubscriptionData {
                                subscription: id,
                                data: data.clone(),
                            });
                        }
                        self.source_cursor = (i + 1) % len;
                    }
                    other => {
                        let error = other
                            .and_then(Result::err)
                            .unwrap_or(EffectError::Unavailable);
                        self.sources.remove(i);
                        self.source_cursor = if self.sources.is_empty() {
                            0
                        } else {
                            i % self.sources.len()
                        };
                        for id in ids {
                            self.pending.push_back(Input::SubscriptionLost {
                                subscription: id,
                                error: error.clone(),
                            });
                        }
                    }
                }
                self.prefer_sources = false;
                return Poll::Ready(self.pending.pop_front().expect("source has subscribers"));
            }
        }
        Poll::Pending
    }
}
async fn execute(resources: Resources, request: RequestId, effect: Effect) -> EffectOutcome {
    effect.validate().map_err(failed)?;
    if let Some(fake) = &resources.fake {
        if let Some(rule) = fake.rule(fake::kind(&effect)) {
            async_io::Timer::after(Duration::from_millis(rule.delay_ms)).await;
            return rule.result;
        }
        match &effect {
            Effect::AudioSetVolume { target, level } => {
                return fake.audio(*target, Some(*level), None);
            }
            Effect::AudioSetMute { target, value } => {
                return fake.audio(*target, None, Some(*value));
            }
            Effect::SpawnRun(_) | Effect::DbusCall(_) => return Ok(EffectResult::None),
            _ => {}
        }
    }
    let command = match effect {
        Effect::DesktopShortcut { keys, app } => Some(DesktopCommand::Shortcut { keys, app }),
        Effect::DesktopBrightness { level } => Some(DesktopCommand::BrightnessSet { level }),
        Effect::DesktopWorkspace { id } => Some(DesktopCommand::WorkspaceActivate { id }),
        Effect::DesktopAppLaunch { id } => Some(DesktopCommand::AppLaunch { id }),
        Effect::AudioSetVolume { target, level } => {
            return resources
                .audio
                .command(AudioCommand::SetVolume { target, level })
                .await;
        }
        Effect::AudioSetMute { target, value } => {
            return resources
                .audio
                .command(AudioCommand::SetMute { target, value })
                .await;
        }
        Effect::TimerAfter { ms } => {
            async_io::Timer::after(Duration::from_millis(ms)).await;
            return Ok(EffectResult::None);
        }
        Effect::SpawnRun(spec) => {
            let native = resources.native.ok_or(EffectError::Unsupported)?;
            let process = start_process(
                &native.spawn,
                &Arc::new(Mutex::new([request.instance].into())),
                spec,
                false,
            )
            .await?;
            loop {
                match process
                    .events
                    .recv()
                    .await
                    .map_err(|_| EffectError::Unavailable)?
                {
                    ProcessEvent::Exited(result) => return result.map(EffectResult::Spawn),
                    ProcessEvent::Line(_) => {}
                }
            }
        }
        Effect::DbusCall(call) => {
            return resources
                .native
                .ok_or(EffectError::Unsupported)?
                .dbus
                .call(&call)
                .await
                .map(EffectResult::Dbus);
        }
        _ => return Err(EffectError::Unsupported),
    };
    if let Some(command) = command {
        resources
            .desktop
            .command(command)
            .await
            .map(|()| EffectResult::None)
            .map_err(|e| match e {
                DesktopError::Unsupported => EffectError::Unsupported,
                DesktopError::Disconnected => EffectError::Unavailable,
                other => failed(other),
            })
    } else {
        Err(EffectError::Unsupported)
    }
}
async fn start_process(
    pool: &SpawnPool,
    owners: &Mutex<BTreeSet<InstanceId>>,
    spec: SpawnSpec,
    lines: bool,
) -> Result<Arc<Process>, EffectError> {
    loop {
        let result = pool.start_shared(&lock(owners).clone(), spec.clone(), lines, None);
        match result {
            Ok(p) => return Ok(Arc::new(p)),
            Err(EffectError::Unavailable) => {
                async_io::Timer::after(Duration::from_millis(5)).await;
            }
            Err(e) => return Err(e),
        }
    }
}
fn make_source(
    resources: Resources,
    kind: SubscriptionKind,
    owners: Arc<Mutex<BTreeSet<InstanceId>>>,
    process: Arc<Mutex<Option<Weak<Process>>>>,
) -> SourceStream {
    if let Err(e) = kind.validate() {
        return Box::pin(stream::once(Err(failed(e))));
    }
    if matches!(kind, SubscriptionKind::DesktopFact { .. }) {
        return Box::pin(stream::pending());
    }
    if let SubscriptionKind::TimerInterval { ms } = kind {
        return Box::pin(stream::unfold(
            Instant::now() + Duration::from_millis(ms),
            move |at| async move {
                async_io::Timer::at(at).await;
                Some((
                    Ok(SubscriptionData::Tick),
                    Instant::now() + Duration::from_millis(ms),
                ))
            },
        ));
    }
    if let Some(fake) = resources.fake {
        let watch = fake.watch(kind);
        return Box::pin(stream::unfold(Some(watch), |watch| async move {
            let watch = watch?;
            let data = watch
                .events
                .recv()
                .await
                .unwrap_or(Err(EffectError::Unavailable));
            let next = if data.is_ok() { Some(watch) } else { None };
            Some((data, next))
        }));
    }
    match kind {
        SubscriptionKind::SpawnLines(spec) => {
            let Some(native) = resources.native else {
                return Box::pin(stream::once(Err(EffectError::Unsupported)));
            };
            Box::pin(stream::unfold(Some(None::<Arc<Process>>), move |state| {
                let native = native.clone();
                let spec = spec.clone();
                let owners = owners.clone();
                let process = process.clone();
                async move {
                    let state = state?;
                    let p = match state {
                        Some(p) => p,
                        None => match start_process(&native.spawn, &owners, spec, true).await {
                            Ok(p) => {
                                *lock(&process) = Some(Arc::downgrade(&p));
                                p
                            }
                            Err(e) => return Some((Err(e), None)),
                        },
                    };
                    match p.events.recv().await {
                        Ok(ProcessEvent::Line(s)) => {
                            Some((Ok(SubscriptionData::Line(s)), Some(Some(p))))
                        }
                        Ok(ProcessEvent::Exited(result)) => Some((
                            Err(match result {
                                Ok(r) => failed(format!("process exited with status {}", r.status)),
                                Err(e) => e,
                            }),
                            None,
                        )),
                        Err(_) => Some((Err(EffectError::Unavailable), None)),
                    }
                }
            }))
        }
        SubscriptionKind::AudioWatch { target } => {
            let watch = resources.audio.watch(target);
            Box::pin(stream::unfold(Some(watch), |state| async move {
                let state = state?;
                let watch = match state {
                    Ok(w) => w,
                    Err(e) => return Some((Err(e), None)),
                };
                let data = watch
                    .events
                    .recv()
                    .await
                    .unwrap_or(Err(EffectError::Unavailable));
                let next = if data.is_ok() { Some(Ok(watch)) } else { None };
                Some((data.map(SubscriptionData::Audio), next))
            }))
        }
        SubscriptionKind::DbusSignal(filter) => {
            let Some(native) = resources.native else {
                return Box::pin(stream::once(Err(EffectError::Unsupported)));
            };
            Box::pin(stream::unfold(Some(None::<dbus::Signals>), move |state| {
                let client = native.dbus.clone();
                let filter = filter.clone();
                async move {
                    let state = state?;
                    let mut handle = match state {
                        Some(h) => h,
                        None => match client.signals(filter).await {
                            Ok(h) => h,
                            Err(e) => return Some((Err(e), None)),
                        },
                    };
                    let result = handle.next().await;
                    let next = if result.is_ok() {
                        Some(Some(handle))
                    } else {
                        None
                    };
                    Some((result.map(SubscriptionData::Dbus), next))
                }
            }))
        }
        SubscriptionKind::DbusProperty(spec) => {
            let Some(native) = resources.native else {
                return Box::pin(stream::once(Err(EffectError::Unsupported)));
            };
            Box::pin(stream::unfold(Some(None::<dbus::Property>), move |state| {
                let client = native.dbus.clone();
                let spec = spec.clone();
                async move {
                    let state = state?;
                    let mut handle = match state {
                        Some(h) => h,
                        None => match client.property(spec).await {
                            Ok(h) => h,
                            Err(e) => return Some((Err(e), None)),
                        },
                    };
                    let result = handle.next().await;
                    let next = if result.is_ok() {
                        Some(Some(handle))
                    } else {
                        None
                    };
                    Some((result.map(SubscriptionData::Dbus), next))
                }
            }))
        }
        _ => Box::pin(stream::once(Err(EffectError::Unsupported))),
    }
}
