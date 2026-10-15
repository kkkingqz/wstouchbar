use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
};
use touchbar_model::*;
#[derive(Clone, Debug)]
pub struct FakeRule {
    pub delay_ms: u64,
    pub result: EffectOutcome,
}
struct Watch {
    id: u64,
    kind: SubscriptionKind,
    sender: async_channel::Sender<Result<SubscriptionData, EffectError>>,
}
struct State {
    rules: BTreeMap<String, FakeRule>,
    watches: Vec<Watch>,
    next: u64,
    audio: [AudioState; 2],
    started: Vec<(RequestId, Effect)>,
}
#[derive(Clone)]
pub struct FakeControl {
    state: Arc<Mutex<State>>,
}
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn index(target: AudioTarget) -> usize {
    match target {
        AudioTarget::DefaultSink => 0,
        AudioTarget::DefaultSource => 1,
    }
}
impl Default for FakeControl {
    fn default() -> Self {
        let state = |name: &str, volume| AudioState {
            available: true,
            volume,
            muted: false,
            name: name.into(),
        };
        Self {
            state: Arc::new(Mutex::new(State {
                rules: BTreeMap::new(),
                watches: vec![],
                next: 0,
                audio: [
                    state("Simulated output", 0.5),
                    state("Simulated microphone", 1.0),
                ],
                started: vec![],
            })),
        }
    }
}
impl FakeControl {
    pub fn set_rule(&self, kind: impl Into<String>, rule: FakeRule) {
        lock(&self.state).rules.insert(kind.into(), rule);
    }
    pub fn started(&self) -> Vec<(RequestId, Effect)> {
        lock(&self.state).started.clone()
    }
    pub fn sources(&self) -> usize {
        lock(&self.state).watches.len()
    }
    pub(crate) fn start(&self, id: RequestId, effect: &Effect) {
        let mut state = lock(&self.state);
        if state.started.len() == 4096 {
            state.started.remove(0);
        }
        state.started.push((id, effect.clone()));
    }
    pub(crate) fn rule(&self, kind: &str) -> Option<FakeRule> {
        lock(&self.state).rules.get(kind).cloned()
    }
    pub fn emit(&self, kind: &SubscriptionKind, data: SubscriptionData) -> Result<(), EffectError> {
        self.publish(kind, Ok(data))
    }
    pub fn lose(&self, kind: &SubscriptionKind, error: EffectError) -> Result<(), EffectError> {
        self.publish(kind, Err(error))
    }
    fn publish(
        &self,
        kind: &SubscriptionKind,
        data: Result<SubscriptionData, EffectError>,
    ) -> Result<(), EffectError> {
        let state = lock(&self.state);
        for watch in &state.watches {
            if &watch.kind == kind && watch.sender.try_send(data.clone()).is_err() {
                watch.sender.close();
                return Err(EffectError::Unavailable);
            }
        }
        Ok(())
    }
    pub(crate) fn audio(
        &self,
        target: AudioTarget,
        volume: Option<f64>,
        mute: Option<MuteValue>,
    ) -> EffectOutcome {
        let mut state = lock(&self.state);
        let audio = &mut state.audio[index(target)];
        if let Some(volume) = volume {
            audio.volume = volume;
        }
        if let Some(mute) = mute {
            audio.muted = match mute {
                MuteValue::On => true,
                MuteValue::Off => false,
                MuteValue::Toggle => !audio.muted,
            };
        }
        let result = if mute.is_some() {
            EffectResult::Mute(audio.muted)
        } else {
            EffectResult::None
        };
        let data = audio.clone();
        drop(state);
        self.emit(
            &SubscriptionKind::AudioWatch { target },
            SubscriptionData::Audio(data),
        )?;
        Ok(result)
    }
    pub(crate) fn watch(&self, kind: SubscriptionKind) -> FakeWatch {
        let (tx, rx) = async_channel::bounded(64);
        let mut state = lock(&self.state);
        state.next = state
            .next
            .checked_add(1)
            .expect("fake source counter exhausted");
        let id = state.next;
        if let SubscriptionKind::AudioWatch { target } = &kind {
            let _ = tx.try_send(Ok(SubscriptionData::Audio(
                state.audio[index(*target)].clone(),
            )));
        }
        state.watches.push(Watch {
            id,
            kind,
            sender: tx,
        });
        FakeWatch {
            events: rx,
            id,
            control: self.clone(),
        }
    }
}
pub(crate) struct FakeWatch {
    pub events: async_channel::Receiver<Result<SubscriptionData, EffectError>>,
    id: u64,
    control: FakeControl,
}
impl Drop for FakeWatch {
    fn drop(&mut self) {
        lock(&self.control.state)
            .watches
            .retain(|w| w.id != self.id);
    }
}
pub(crate) fn kind(effect: &Effect) -> &'static str {
    match effect {
        Effect::AudioSetVolume { .. } => "audio.set-volume",
        Effect::AudioSetMute { .. } => "audio.set-mute",
        Effect::MediaPlayPause => "media.play-pause",
        Effect::MediaNext => "media.next",
        Effect::MediaPrevious => "media.previous",
        Effect::MediaSeek { .. } => "media.seek",
        Effect::DesktopShortcut { .. } => "desktop.shortcut",
        Effect::DesktopBrightness { .. } => "desktop.brightness.set",
        Effect::DesktopWorkspace { .. } => "desktop.workspace.activate",
        Effect::DesktopAppLaunch { .. } => "desktop.app.launch",
        Effect::KeyTap { .. } => "key.tap",
        Effect::KeyDown { .. } => "key.down",
        Effect::KeyUp { .. } => "key.up",
        Effect::OpenUri { .. } => "open.uri",
        Effect::TimerAfter { .. } => "timer.after",
        Effect::SpawnRun(_) => "spawn.run",
        Effect::DbusCall(_) => "dbus.call",
    }
}
