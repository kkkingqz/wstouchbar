use crate::*;
use std::collections::{BTreeMap, BTreeSet};
use touchbar_core::*;
use touchbar_model::*;
#[derive(Clone)]
enum Guard {
    Always,
    Logic(InstanceId, CallId),
    Effect(RequestId),
    Subscription(SubscriptionId),
    Timer(TimerId),
}
#[derive(Clone)]
struct Job {
    at: Timestamp,
    sequence: u64,
    input: Input,
    guard: Guard,
    repeat: Option<u64>,
}
pub struct Harness {
    core: Core,
    clock: Timestamp,
    logic_timeout_ms: u64,
    next_job: u64,
    jobs: Vec<Job>,
    core_timer: Option<TimerId>,
    pending_calls: BTreeSet<(InstanceId, CallId)>,
    runtime_sets: BTreeMap<InstanceId, SetRef>,
    contacts: BTreeMap<(SurfaceId, ContactId), Point>,
    stored: Vec<(PersistKey, StatePatch)>,
    recorder: Recorder,
    pub trace: Vec<TraceEntry>,
    pub logic: FakeLogic,
    pub effects: FakeEffects,
    pub renderer: FakeRenderer,
}
impl Harness {
    pub fn new(config: CoreConfig) -> Self {
        let logic_timeout_ms = config.logic_hard_ms;
        let core = Core::new(config);
        let recorder = Recorder::new(&core);
        Self {
            core,
            recorder,
            clock: Timestamp(0),
            logic_timeout_ms,
            next_job: 0,
            jobs: vec![],
            core_timer: None,
            pending_calls: BTreeSet::new(),
            runtime_sets: BTreeMap::new(),
            contacts: BTreeMap::new(),
            stored: vec![],
            trace: vec![],
            logic: FakeLogic::default(),
            effects: FakeEffects::default(),
            renderer: FakeRenderer::default(),
        }
    }
    pub fn now(&self) -> Timestamp {
        self.clock
    }
    pub fn core(&self) -> &Core {
        &self.core
    }
    pub fn snapshot(&self) -> CoreSnapshot {
        self.core.snapshot()
    }
    pub fn dump(&self) -> ReplayDump {
        self.recorder.dump()
    }
    pub fn send(&mut self, input: Input) -> Vec<Output> {
        let start = self.trace.len();
        self.deliver(input);
        self.flush_until(self.clock);
        self.trace[start..]
            .iter()
            .flat_map(|e| e.outputs.clone())
            .collect()
    }
    pub fn advance_ms(&mut self, ms: u64) {
        self.flush_until(self.clock.after(ms));
    }
    fn schedule(&mut self, at: Timestamp, input: Input, guard: Guard, repeat: Option<u64>) {
        self.jobs.push(Job {
            at: at.max(self.clock),
            sequence: self.next_job,
            input,
            guard,
            repeat,
        });
        self.next_job += 1;
    }
    fn flush_until(&mut self, end: Timestamp) {
        let mut iterations = 0;
        loop {
            let next = self
                .jobs
                .iter()
                .enumerate()
                .filter(|(_, j)| j.at <= end)
                .min_by_key(|(_, j)| (j.at, j.sequence))
                .map(|(i, _)| i);
            let Some(index) = next else {
                break;
            };
            let job = self.jobs.remove(index);
            self.clock = job.at;
            iterations += 1;
            assert!(
                iterations <= 100_000,
                "fake executor produced an endless event loop"
            );
            let active = match job.guard {
                Guard::Always => true,
                Guard::Logic(instance, call) => self.pending_calls.remove(&(instance, call)),
                Guard::Effect(request) => self.effects.active.remove(&request),
                Guard::Subscription(subscription) => {
                    self.effects.subscriptions.contains_key(&subscription)
                }
                Guard::Timer(timer) => {
                    if self.core_timer == Some(timer) {
                        self.core_timer = None;
                        true
                    } else {
                        false
                    }
                }
            };
            if !active {
                continue;
            }
            self.deliver(job.input.clone());
            if let Some(ms) = job.repeat
                && let Guard::Subscription(subscription) = job.guard
                && self.effects.subscriptions.contains_key(&subscription)
            {
                self.schedule(self.clock.after(ms), job.input, job.guard, Some(ms));
            }
        }
        self.clock = end;
    }
    fn deliver(&mut self, input: Input) {
        if let Input::SubscriptionLost { subscription, .. } = &input {
            self.effects.subscriptions.remove(subscription);
        }
        if let Input::Layout {
            surface, hit_map, ..
        } = &input
        {
            self.renderer.hits.insert(*surface, hit_map.clone());
        }
        let outputs = self
            .recorder
            .handle(&mut self.core, self.clock, input.clone());
        self.trace.push(TraceEntry {
            at: self.clock,
            input,
            outputs: outputs.clone(),
        });
        for command in outputs {
            self.command(command);
        }
    }
    fn command(&mut self, command: Output) {
        match command {
            Output::CreateInstance {
                instance,
                set,
                persist_key,
                ..
            } => {
                let spec = self
                    .core
                    .snapshot()
                    .surfaces
                    .into_iter()
                    .flat_map(|s| s.scene.items)
                    .find(|i| i.instance == instance)
                    .expect("created instance in scene")
                    .set;
                let description = self.logic.description(&spec);
                self.runtime_sets.insert(instance, set);
                let persisted = self
                    .stored
                    .iter()
                    .find(|(key, _)| key == &persist_key)
                    .map(|(_, fields)| fields.clone())
                    .unwrap_or_default();
                self.schedule(
                    self.clock,
                    Input::LogicDescribed {
                        instance,
                        description,
                        persisted,
                    },
                    Guard::Always,
                    None,
                );
            }
            Output::DisposeInstance { instance } => {
                self.runtime_sets.remove(&instance);
                self.pending_calls.retain(|(id, _)| *id != instance);
            }
            Output::CallLogic {
                instance,
                call,
                invocation,
            } => {
                let Some(set) = self.runtime_sets.get(&instance) else {
                    return;
                };
                self.pending_calls.insert((instance, call));
                let reaction = self.logic.call(set, instance, call, &invocation);
                let (ms, input) = match reaction {
                    LogicReaction::Update(update) => (
                        0,
                        Some(Input::LogicUpdate {
                            instance,
                            call,
                            update,
                        }),
                    ),
                    LogicReaction::Delayed { ms, update } => (
                        ms,
                        Some(Input::LogicUpdate {
                            instance,
                            call,
                            update,
                        }),
                    ),
                    LogicReaction::Error(message) => (
                        0,
                        Some(Input::LogicError {
                            instance,
                            call,
                            message,
                        }),
                    ),
                    LogicReaction::Fault(reason) => (
                        0,
                        Some(Input::LogicFault {
                            instance,
                            call,
                            reason,
                        }),
                    ),
                    LogicReaction::Hang => (0, None),
                };
                if let Some(input) = input {
                    self.schedule(
                        self.clock.after(ms),
                        input,
                        Guard::Logic(instance, call),
                        None,
                    );
                }
                let timeout = self.logic_timeout_ms;
                self.schedule(
                    self.clock.after(timeout),
                    Input::LogicFault {
                        instance,
                        call,
                        reason: "fake logic timeout".into(),
                    },
                    Guard::Logic(instance, call),
                    None,
                );
            }
            Output::StartEffect {
                request, effect, ..
            } => {
                let (ms, result) = self.effects.start(request, effect);
                self.schedule(
                    self.clock.after(ms),
                    Input::EffectDone { request, result },
                    Guard::Effect(request),
                    None,
                );
            }
            Output::CancelEffect { request } => self.effects.cancel(request),
            Output::Subscribe { subscription, kind } => {
                self.effects
                    .subscriptions
                    .insert(subscription, kind.clone());
                if let SubscriptionKind::TimerInterval { ms } = kind {
                    self.schedule(
                        self.clock.after(ms),
                        Input::SubscriptionData {
                            subscription,
                            data: SubscriptionData::Tick,
                        },
                        Guard::Subscription(subscription),
                        Some(ms),
                    );
                }
            }
            Output::Unsubscribe { subscription } => {
                self.effects.subscriptions.remove(&subscription);
            }
            Output::Render {
                surface,
                generation,
                scene,
            } => {
                let info = self
                    .core
                    .snapshot()
                    .surfaces
                    .into_iter()
                    .find(|s| s.info.id == surface)
                    .expect("render surface")
                    .info;
                let layout = self.renderer.render(&info, generation, &scene);
                let at = self.clock.after(self.renderer.delay_ms);
                self.schedule(at, layout, Guard::Always, None);
                self.schedule(
                    at,
                    Input::FrameDone {
                        surface,
                        generation,
                    },
                    Guard::Always,
                    None,
                );
            }
            Output::SetTimer { id, at } => {
                self.core_timer = Some(id);
                self.schedule(at, Input::Timer { id }, Guard::Timer(id), None);
            }
            Output::ClearTimer { id } => {
                if self.core_timer == Some(id) {
                    self.core_timer = None;
                }
            }
            Output::Persist { key, fields, .. } => {
                let index = self
                    .stored
                    .iter()
                    .position(|(k, _)| k == &key)
                    .unwrap_or_else(|| {
                        self.stored.push((key, vec![]));
                        self.stored.len() - 1
                    });
                for (name, value) in fields {
                    let saved = &mut self.stored[index].1;
                    if let Some((_, old)) = saved.iter_mut().find(|(n, _)| n == &name) {
                        *old = value;
                    } else {
                        saved.push((name, value));
                    }
                }
            }
            Output::SendKey { .. }
            | Output::DesktopKey { .. }
            | Output::WatchKeys { .. }
            | Output::Backlight { .. }
            | Output::Log { .. } => {}
        }
    }
    pub fn subscription_after(
        &mut self,
        subscription: SubscriptionId,
        data: SubscriptionData,
        ms: u64,
    ) {
        self.schedule(
            self.clock.after(ms),
            Input::SubscriptionData { subscription, data },
            Guard::Subscription(subscription),
            None,
        );
    }
    pub fn hit(&self, surface: SurfaceId, widget: &str) -> Point {
        assert!(
            self.core
                .snapshot()
                .surfaces
                .iter()
                .any(|s| s.info.id == surface
                    && s.mode == Mode::Display
                    && s.layout_generation == Some(s.generation)),
            "surface has no current display layout"
        );
        let hit = self
            .renderer
            .hits
            .get(&surface)
            .into_iter()
            .flatten()
            .filter(|h| h.widget.as_str() == widget)
            .max_by_key(|h| h.plane)
            .unwrap_or_else(|| panic!("widget {widget} has no hit on {surface:?}"));
        Point {
            x: hit.rect.x + hit.rect.width / 2.0,
            y: hit.rect.y + hit.rect.height / 2.0,
        }
    }
    pub fn touch(&mut self, surface: SurfaceId, contact: u32, phase: TouchPhase, point: Point) {
        let contact = ContactId::new(contact).expect("nonzero contact id");
        if matches!(phase, TouchPhase::Down | TouchPhase::Move) {
            self.contacts.insert((surface, contact), point);
        } else {
            self.contacts.remove(&(surface, contact));
        }
        self.send(Input::Touch {
            surface,
            contact,
            phase,
            x: point.x,
            y: point.y,
        });
    }
    pub fn touch_up(&mut self, surface: SurfaceId, contact: u32) {
        let id = ContactId::new(contact).expect("nonzero contact id");
        let point = *self
            .contacts
            .get(&(surface, id))
            .expect("contact down before up");
        self.touch(surface, contact, TouchPhase::Up, point);
    }
    pub fn tap(&mut self, surface: SurfaceId, widget: &str) {
        let point = self.hit(surface, widget);
        self.touch(surface, 1, TouchPhase::Down, point);
        self.touch_up(surface, 1);
    }
}
