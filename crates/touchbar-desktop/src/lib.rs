//! Serializable desktop contract and deterministic in-memory backends.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard},
};
use touchbar_model::*;
pub type CommandFuture = Pin<Box<dyn Future<Output = Result<(), DesktopError>> + Send + 'static>>;
pub trait DesktopBackend: Send {
    fn name(&self) -> &'static str;
    fn start(&mut self, events: async_channel::Sender<BackendEvent>) -> Result<(), DesktopError>;
    fn command(&self, command: DesktopCommand) -> CommandFuture;
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub facts: BTreeSet<FactName>,
    pub commands: BTreeSet<CommandKind>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandKind {
    Shortcut,
    Key,
    BrightnessSet,
    WorkspaceActivate,
    AppLaunch,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DesktopCommand {
    Shortcut {
        keys: Vec<String>,
        app: Option<String>,
    },
    Key {
        key: SystemKey,
        phase: KeyPhase,
    },
    BrightnessSet {
        #[serde(with = "touchbar_model::wire_float::f64")]
        level: f64,
    },
    WorkspaceActivate {
        id: String,
    },
    AppLaunch {
        id: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BackendEvent {
    Ready(Capabilities),
    Fact(FactName, Option<FactValue>),
    Lost(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum DesktopError {
    #[error("unsupported")]
    Unsupported,
    #[error("desktop disconnected")]
    Disconnected,
    #[error("desktop event queue is full or closed")]
    EventQueue,
    #[error("invalid desktop argument: {0}")]
    Invalid(String),
    #[error("desktop already started")]
    AlreadyStarted,
}
pub type FactChange = (FactName, Option<FactValue>);
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackendStatus {
    #[default]
    Connecting,
    Ready,
    Lost {
        reason: String,
    },
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BackendDelta {
    pub facts: Vec<FactChange>,
    pub capabilities: Option<Capabilities>,
}
#[derive(Clone, Debug)]
pub struct ReconnectBackoff {
    delay: u64,
}
impl Default for ReconnectBackoff {
    fn default() -> Self {
        Self { delay: 1000 }
    }
}
impl ReconnectBackoff {
    pub fn failed(&mut self, now: Timestamp) -> Timestamp {
        let retry = Timestamp(now.0.saturating_add(self.delay));
        self.delay = self.delay.saturating_mul(2).min(60_000);
        retry
    }
    pub fn reset(&mut self) {
        self.delay = 1000;
    }
}
/// Shell bookkeeping. Clearing uses only names owned by this backend, never unrelated facts.
#[derive(Clone, Debug, Default)]
pub struct BackendMonitor {
    status: BackendStatus,
    capabilities: Capabilities,
    facts: Facts,
    retry_at: Option<Timestamp>,
    backoff: ReconnectBackoff,
}
impl BackendMonitor {
    pub fn status(&self) -> &BackendStatus {
        &self.status
    }
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }
    pub fn facts(&self) -> &Facts {
        &self.facts
    }
    pub fn retry_at(&self) -> Option<Timestamp> {
        self.retry_at
    }
    pub fn begin_reconnect(&mut self, now: Timestamp) -> bool {
        if self.retry_at.is_some_and(|t| t <= now) {
            self.retry_at = None;
            self.status = BackendStatus::Connecting;
            true
        } else {
            false
        }
    }
    pub fn apply(
        &mut self,
        event: BackendEvent,
        now: Timestamp,
    ) -> Result<BackendDelta, DesktopError> {
        let mut delta = BackendDelta::default();
        match event {
            BackendEvent::Ready(capabilities) => {
                self.status = BackendStatus::Ready;
                self.retry_at = None;
                self.backoff.reset();
                self.capabilities = capabilities.clone();
                delta.capabilities = Some(capabilities);
            }
            BackendEvent::Lost(reason) => {
                self.status = BackendStatus::Lost { reason };
                self.retry_at = Some(self.backoff.failed(now));
                self.capabilities = Capabilities::default();
                delta.capabilities = Some(Capabilities::default());
                delta.facts = std::mem::take(&mut self.facts)
                    .into_keys()
                    .map(|n| (n, None))
                    .collect();
            }
            BackendEvent::Fact(name, value) => {
                if self.status != BackendStatus::Ready {
                    return Ok(delta);
                }
                validate_fact(&name, value.as_ref())?;
                if value.is_some() && !self.facts.contains_key(&name) && self.facts.len() >= 256 {
                    return Err(DesktopError::Invalid("too many facts".into()));
                }
                if let Some(new_value) = &value {
                    self.facts.insert(name.clone(), new_value.clone());
                    delta.facts.push((name, value));
                } else if self.facts.remove(&name).is_some() {
                    delta.facts.push((name, None));
                }
            }
        }
        Ok(delta)
    }
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.chars().any(|c| c.is_control() || c.is_whitespace())
}
fn validate_fact(name: &FactName, value: Option<&FactValue>) -> Result<(), DesktopError> {
    if !valid_name(&name.0) || name.0.starts_with("surface.") {
        return Err(DesktopError::Invalid(
            "invalid or reserved fact name".into(),
        ));
    }
    if let Some(value) = value {
        value
            .validate()
            .map_err(|e| DesktopError::Invalid(e.to_string()))?;
        let bytes = match value {
            FactValue::Boolean(_) | FactValue::Number(_) => 8,
            FactValue::Text(s) => s.len(),
            FactValue::List(v) => {
                if v.len() > 256 {
                    return Err(DesktopError::Invalid("too many list entries".into()));
                }
                v.iter().map(String::len).sum()
            }
            FactValue::Records(v) => {
                if v.len() > 256
                    || v.iter()
                        .any(|r| r.len() > 32 || r.keys().any(|n| !valid_name(n)))
                {
                    return Err(DesktopError::Invalid("record budget exceeded".into()));
                }
                v.iter()
                    .flat_map(|r| r.iter())
                    .map(|(n, v)| {
                        n.len()
                            + match v {
                                Scalar::Text(s) => s.len(),
                                _ => 8,
                            }
                    })
                    .sum()
            }
        };
        if bytes > 256 * 1024 {
            return Err(DesktopError::Invalid("fact exceeds 256 KiB".into()));
        }
    }
    Ok(())
}
#[derive(Default)]
pub struct NullBackend {
    started: bool,
}
impl DesktopBackend for NullBackend {
    fn name(&self) -> &'static str {
        "null"
    }
    fn start(&mut self, events: async_channel::Sender<BackendEvent>) -> Result<(), DesktopError> {
        if self.started {
            return Err(DesktopError::AlreadyStarted);
        }
        events
            .try_send(BackendEvent::Ready(Capabilities::default()))
            .map_err(|_| DesktopError::EventQueue)?;
        self.started = true;
        Ok(())
    }
    fn command(&self, _: DesktopCommand) -> CommandFuture {
        Box::pin(std::future::ready(Err(DesktopError::Unsupported)))
    }
}
struct FakeState {
    events: Option<async_channel::Sender<BackendEvent>>,
    ready: bool,
    facts: Facts,
    log: Vec<DesktopCommand>,
}
/// Clones share simulated state; no system command, compositor or D-Bus access occurs.
#[derive(Clone)]
pub struct FakeBackend {
    shared: Arc<Mutex<FakeState>>,
}
impl Default for FakeBackend {
    fn default() -> Self {
        Self {
            shared: Arc::new(Mutex::new(FakeState {
                events: None,
                ready: false,
                facts: BTreeMap::from([
                    ("appearance.scheme".into(), FactValue::Text("dark".into())),
                    ("display.brightness".into(), FactValue::Number(1.0)),
                ]),
                log: vec![],
            })),
        }
    }
}
impl FakeBackend {
    fn state(&self) -> MutexGuard<'_, FakeState> {
        self.shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub fn fact(&self, name: &FactName) -> Option<FactValue> {
        self.state().facts.get(name).cloned()
    }
    pub fn log(&self) -> Vec<DesktopCommand> {
        self.state().log.clone()
    }
    pub fn take_log(&self) -> Vec<DesktopCommand> {
        std::mem::take(&mut self.state().log)
    }
    pub fn set_fact(&self, name: FactName, value: Option<FactValue>) -> Result<(), DesktopError> {
        validate_fact(&name, value.as_ref())?;
        let mut state = self.state();
        set_fact(&mut state, name, value)
    }
    pub fn lose(&self, reason: impl Into<String>) -> Result<(), DesktopError> {
        let mut state = self.state();
        let events = state.events.as_ref().ok_or(DesktopError::Disconnected)?;
        events
            .try_send(BackendEvent::Lost(reason.into()))
            .map_err(|_| DesktopError::EventQueue)?;
        state.ready = false;
        Ok(())
    }
    pub fn reconnect(&self) -> Result<(), DesktopError> {
        let mut state = self.state();
        if state.ready {
            return Ok(());
        }
        announce(&mut state)
    }
    fn execute(&self, command: DesktopCommand) -> Result<(), DesktopError> {
        let mut state = self.state();
        if !state.ready {
            return Err(DesktopError::Disconnected);
        }
        match &command {
            DesktopCommand::BrightnessSet { level } => {
                if !level.is_finite() || !(0.0..=1.0).contains(level) {
                    return Err(DesktopError::Invalid("brightness must be in 0..=1".into()));
                }
                set_fact(
                    &mut state,
                    "display.brightness".into(),
                    Some(FactValue::Number(*level)),
                )?;
            }
            DesktopCommand::Shortcut { keys, app } => {
                if keys.is_empty()
                    || keys.len() > 16
                    || keys.iter().any(|k| !valid_name(k))
                    || app.as_ref().is_some_and(|s| !valid_name(s))
                {
                    return Err(DesktopError::Invalid("invalid shortcut".into()));
                }
            }
            DesktopCommand::WorkspaceActivate { id } | DesktopCommand::AppLaunch { id } => {
                if !valid_name(id) {
                    return Err(DesktopError::Invalid("invalid id".into()));
                }
            }
            DesktopCommand::Key { .. } => {}
        }
        if state.log.len() == 1024 {
            state.log.remove(0);
        }
        state.log.push(command);
        Ok(())
    }
}
fn set_fact(
    state: &mut FakeState,
    name: FactName,
    value: Option<FactValue>,
) -> Result<(), DesktopError> {
    if value.is_some() && !state.facts.contains_key(&name) && state.facts.len() >= 256 {
        return Err(DesktopError::Invalid("too many facts".into()));
    }
    if state.ready {
        state
            .events
            .as_ref()
            .ok_or(DesktopError::Disconnected)?
            .try_send(BackendEvent::Fact(name.clone(), value.clone()))
            .map_err(|_| DesktopError::EventQueue)?;
    }
    if let Some(value) = value {
        state.facts.insert(name, value);
    } else {
        state.facts.remove(&name);
    }
    Ok(())
}
fn announce(state: &mut FakeState) -> Result<(), DesktopError> {
    let events = state.events.as_ref().ok_or(DesktopError::Disconnected)?;
    if events.is_closed()
        || events
            .capacity()
            .is_some_and(|cap| cap.saturating_sub(events.len()) < state.facts.len() + 1)
    {
        return Err(DesktopError::EventQueue);
    }
    let caps = Capabilities {
        facts: state.facts.keys().cloned().collect(),
        commands: [
            CommandKind::Shortcut,
            CommandKind::Key,
            CommandKind::BrightnessSet,
            CommandKind::WorkspaceActivate,
            CommandKind::AppLaunch,
        ]
        .into(),
    };
    events
        .try_send(BackendEvent::Ready(caps))
        .map_err(|_| DesktopError::EventQueue)?;
    for (name, value) in &state.facts {
        events
            .try_send(BackendEvent::Fact(name.clone(), Some(value.clone())))
            .map_err(|_| DesktopError::EventQueue)?;
    }
    state.ready = true;
    Ok(())
}
impl DesktopBackend for FakeBackend {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn start(&mut self, events: async_channel::Sender<BackendEvent>) -> Result<(), DesktopError> {
        let mut state = self.state();
        if state.events.is_some() {
            return Err(DesktopError::AlreadyStarted);
        }
        state.events = Some(events);
        if let Err(error) = announce(&mut state) {
            state.events = None;
            return Err(error);
        }
        Ok(())
    }
    fn command(&self, command: DesktopCommand) -> CommandFuture {
        Box::pin(std::future::ready(self.execute(command)))
    }
}
