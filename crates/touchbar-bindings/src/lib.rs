//! Deterministic, serializable key binding state machines.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub use touchbar_model::{
    Binding, BindingIssue, KeyGesture, KeyTrigger, Trigger, validate_bindings,
};
use touchbar_model::{Facts, KeyPhase, Timestamp, WatchKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyInput {
    pub code: WatchKey,
    pub phase: KeyPhase,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BindingEvent<A> {
    Fired { id: String, action: A },
    Began { id: String, action: A },
    Ended { id: String, action: A },
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct GestureState {
    started: Option<Timestamp>,
    interrupted: bool,
    active: bool,
    fired: bool,
    taps: u32,
    window: Option<Timestamp>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Progress {
    step: usize,
    deadline: Option<Timestamp>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry<A> {
    binding: Binding<A>,
    enabled: bool,
    gestures: Vec<GestureState>,
    progress: Progress,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BindingEngine<A> {
    entries: Vec<Entry<A>>,
    pressed: BTreeSet<WatchKey>,
    facts: Facts,
}

impl<A: Clone + PartialEq> BindingEngine<A> {
    pub fn new(bindings: Vec<Binding<A>>) -> Result<Self, Vec<BindingIssue>> {
        let issues = validate_bindings(&bindings);
        if !issues.is_empty() {
            return Err(issues);
        }
        let facts = Facts::new();
        let entries = bindings
            .into_iter()
            .map(|binding| {
                let enabled = binding.when.as_ref().is_none_or(|c| c.evaluate(&facts));
                let count = match &binding.trigger {
                    Trigger::Key(_) => 1,
                    Trigger::Sequence { steps, .. } => steps.len(),
                };
                Entry {
                    binding,
                    enabled,
                    gestures: vec![GestureState::default(); count],
                    progress: Progress::default(),
                }
            })
            .collect();
        Ok(Self {
            entries,
            pressed: BTreeSet::new(),
            facts,
        })
    }
    pub fn handle(&mut self, now: Timestamp, input: KeyInput) -> Vec<BindingEvent<A>> {
        let mut events = self.advance(now);
        let changed = match input.phase {
            KeyPhase::Pressed => self.pressed.insert(input.code),
            KeyPhase::Released => self.pressed.remove(&input.code),
        };
        if !changed {
            return events;
        }
        for entry in &mut self.entries {
            if !entry.enabled {
                continue;
            }
            if let Trigger::Sequence { steps, .. } = &entry.binding.trigger
                && entry.progress.step > 0
                && input.phase == KeyPhase::Pressed
                && !steps[entry.progress.step].keys.contains(&input.code)
            {
                // Reset before processing this key, so it may begin a fresh first step.
                entry.reset(&mut events);
            }
            let signals = key_triggers(&entry.binding.trigger)
                .iter()
                .zip(&mut entry.gestures)
                .map(|(trigger, state)| state.key(now, input, &self.pressed, trigger))
                .collect();
            entry.consume(now, signals, &mut events);
        }
        events
    }
    /// Deliver all elapsed deadlines chronologically; no wall-clock reads.
    pub fn advance(&mut self, now: Timestamp) -> Vec<BindingEvent<A>> {
        let mut events = vec![];
        while let Some(at) = self.next_deadline().filter(|at| *at <= now) {
            for entry in &mut self.entries {
                if !entry.enabled {
                    continue;
                }
                if entry
                    .progress
                    .deadline
                    .is_some_and(|deadline| deadline <= at)
                {
                    entry.progress = Progress::default();
                }
                let signals = key_triggers(&entry.binding.trigger)
                    .iter()
                    .zip(&mut entry.gestures)
                    .map(|(trigger, state)| state.timer(at, trigger))
                    .collect();
                entry.consume(at, signals, &mut events);
            }
        }
        events
    }
    /// Conditions are updated before due timers: disabling at a deadline cancels it.
    pub fn set_facts(&mut self, now: Timestamp, facts: Facts) -> Vec<BindingEvent<A>> {
        let mut events = vec![];
        self.facts = facts;
        for entry in &mut self.entries {
            let enabled = entry
                .binding
                .when
                .as_ref()
                .is_none_or(|c| c.evaluate(&self.facts));
            if entry.enabled && !enabled {
                entry.reset(&mut events);
            }
            entry.enabled = enabled;
        }
        events.extend(self.advance(now));
        events
    }
    pub fn next_deadline(&self) -> Option<Timestamp> {
        self.entries
            .iter()
            .filter(|e| e.enabled)
            .flat_map(|entry| {
                key_triggers(&entry.binding.trigger)
                    .iter()
                    .zip(&entry.gestures)
                    .filter_map(|(trigger, state)| state.deadline(trigger))
                    .chain(entry.progress.deadline)
                    .collect::<Vec<_>>()
            })
            .min()
    }
    pub fn reset(&mut self) -> Vec<BindingEvent<A>> {
        let mut events = vec![];
        self.pressed.clear();
        for entry in &mut self.entries {
            entry.reset(&mut events);
        }
        events
    }
    pub fn watch_keys(&self) -> BTreeSet<WatchKey> {
        self.entries
            .iter()
            .flat_map(|e| e.binding.trigger.watch_keys())
            .collect()
    }
}

fn key_triggers(trigger: &Trigger) -> &[KeyTrigger] {
    match trigger {
        Trigger::Key(key) => std::slice::from_ref(key),
        Trigger::Sequence { steps, .. } => steps,
    }
}
#[derive(Clone, Copy)]
enum Signal {
    Fired,
    Began,
    Ended,
}
impl<A: Clone> Entry<A> {
    fn emit(&self, signal: Signal, events: &mut Vec<BindingEvent<A>>) {
        let id = self.binding.id.clone();
        let action = self.binding.action.clone();
        events.push(match signal {
            Signal::Fired => BindingEvent::Fired { id, action },
            Signal::Began => BindingEvent::Began { id, action },
            Signal::Ended => BindingEvent::Ended { id, action },
        });
    }
    fn reset(&mut self, events: &mut Vec<BindingEvent<A>>) {
        if self.gestures.iter().any(|s| s.active) {
            self.emit(Signal::Ended, events);
        }
        self.gestures.fill(GestureState::default());
        self.progress = Progress::default();
    }
    fn consume(
        &mut self,
        now: Timestamp,
        signals: Vec<Option<Signal>>,
        events: &mut Vec<BindingEvent<A>>,
    ) {
        match &self.binding.trigger {
            Trigger::Key(_) => {
                if let Some(signal) = signals[0] {
                    self.emit(signal, events);
                }
            }
            Trigger::Sequence { steps, within_ms } => {
                if matches!(signals[self.progress.step], Some(Signal::Fired)) {
                    if self.progress.step == 0 {
                        self.progress.deadline = Some(now.after(*within_ms));
                    }
                    self.progress.step += 1;
                    if self.progress.step == steps.len() {
                        self.progress = Progress::default();
                        self.emit(Signal::Fired, events);
                    }
                } else if matches!(signals[0], Some(Signal::Fired)) {
                    self.progress.step = 1;
                    self.progress.deadline = Some(now.after(*within_ms));
                }
            }
        }
    }
}

impl GestureState {
    fn deadline(&self, trigger: &KeyTrigger) -> Option<Timestamp> {
        let armed = self.started.and_then(|start| match trigger.gesture {
            KeyGesture::Hold { ms } if !self.fired => Some(start.after(ms)),
            KeyGesture::Tap { ms } if !self.interrupted => Some(start.after(ms)),
            KeyGesture::MultiTap { tap_ms, .. } if !self.interrupted => Some(start.after(tap_ms)),
            _ => None,
        });
        let window = match trigger.gesture {
            KeyGesture::MultiTap { within_ms, .. } => {
                self.window.map(|start| start.after(within_ms))
            }
            _ => None,
        };
        armed.into_iter().chain(window).min()
    }
    fn key(
        &mut self,
        now: Timestamp,
        input: KeyInput,
        pressed: &BTreeSet<WatchKey>,
        trigger: &KeyTrigger,
    ) -> Option<Signal> {
        let member = trigger.keys.contains(&input.code);
        if input.phase == KeyPhase::Pressed && !member {
            self.interrupted = true;
            if matches!(trigger.gesture, KeyGesture::MultiTap { .. }) {
                self.taps = 0;
                self.window = None;
            }
        }
        if input.phase == KeyPhase::Pressed
            && member
            && trigger.keys.iter().all(|key| pressed.contains(key))
        {
            self.started = Some(now);
            self.fired = false;
            self.interrupted = pressed.iter().any(|key| !trigger.keys.contains(key));
            match trigger.gesture {
                KeyGesture::Press => return Some(Signal::Fired),
                KeyGesture::WhileHeld => {
                    self.active = true;
                    return Some(Signal::Began);
                }
                KeyGesture::MultiTap { .. } => {
                    self.window.get_or_insert(now);
                }
                _ => {}
            }
        }
        if input.phase != KeyPhase::Released || !member {
            return None;
        }
        let start = self.started.take()?;
        self.fired = false;
        match trigger.gesture {
            KeyGesture::Release => Some(Signal::Fired),
            KeyGesture::WhileHeld => {
                self.active = false;
                Some(Signal::Ended)
            }
            KeyGesture::Tap { ms } if !self.interrupted && now.elapsed_since(start) < ms => {
                Some(Signal::Fired)
            }
            KeyGesture::MultiTap { count, tap_ms, .. }
                if !self.interrupted && now.elapsed_since(start) < tap_ms =>
            {
                self.taps += 1;
                if self.taps == count {
                    self.taps = 0;
                    self.window = None;
                    Some(Signal::Fired)
                } else {
                    None
                }
            }
            KeyGesture::MultiTap { .. } => {
                self.taps = 0;
                self.window = None;
                None
            }
            _ => None,
        }
    }
    fn timer(&mut self, now: Timestamp, trigger: &KeyTrigger) -> Option<Signal> {
        if let KeyGesture::MultiTap { within_ms, .. } = trigger.gesture
            && self
                .window
                .is_some_and(|start| start.after(within_ms) <= now)
        {
            *self = Self::default();
            return None;
        }
        let start = self.started?;
        match trigger.gesture {
            KeyGesture::Hold { ms } if !self.fired && start.after(ms) <= now => {
                self.fired = true;
                Some(Signal::Fired)
            }
            KeyGesture::Tap { ms } if start.after(ms) <= now => {
                self.started = None;
                None
            }
            KeyGesture::MultiTap { tap_ms, .. } if start.after(tap_ms) <= now => {
                *self = Self::default();
                None
            }
            _ => None,
        }
    }
}

pub mod config;
