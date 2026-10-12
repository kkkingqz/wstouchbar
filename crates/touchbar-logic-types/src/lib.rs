//! Shared host and guest limits. Wire costs are independent of Rust layout and JSON.
use std::fmt;
use touchbar_model::*;

/// Deterministic clock supplied by the caller, in monotonic session milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallContext {
    pub now_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogicLimits {
    pub logic_soft_ms: u64,
    pub logic_hard_ms: u64,
    pub max_memory_bytes: usize,
    pub max_wasm_stack_bytes: usize,
    pub max_update_bytes: usize,
    pub max_requests_per_update: usize,
    pub max_subscriptions: usize,
    pub max_text_bytes: usize,
    pub max_records: usize,
    pub max_persist_bytes: usize,
    pub max_log_records: usize,
    pub max_log_message_bytes: usize,
}

impl Default for LogicLimits {
    fn default() -> Self {
        Self {
            logic_soft_ms: 5,
            logic_hard_ms: 50,
            max_memory_bytes: 16 * 1024 * 1024,
            max_wasm_stack_bytes: 512 * 1024,
            max_update_bytes: 256 * 1024,
            max_requests_per_update: 64,
            max_subscriptions: 32,
            max_text_bytes: 4 * 1024,
            max_records: 1000,
            max_persist_bytes: 64 * 1024,
            max_log_records: 64,
            max_log_message_bytes: 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetExceeded {
    pub actual: usize,
    pub limit: usize,
}
impl fmt::Display for BudgetExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "logic update budget exceeded: {} bytes > {} bytes",
            self.actual, self.limit
        )
    }
}
impl std::error::Error for BudgetExceeded {}

/// A conservative estimate: header 64, list entries 16, strings 8 + UTF-8 bytes,
/// scalar discriminants 8. Fixed canvas commands cost 64, path segments 32.
/// Saturating arithmetic prevents a wrapped size from bypassing the budget.
pub fn wire_size(update: &LogicUpdate) -> usize {
    let mut size = 64usize;
    for (key, value) in &update.state {
        size = size.saturating_add(total([16, text(key), state_value(value)]));
    }
    for request in &update.requests {
        size = size.saturating_add(total([
            32,
            text(&request.tag),
            effect(&request.effect),
            optional_text(&request.coalesce),
        ]));
    }
    for tag in &update.cancel {
        size = size.saturating_add(total([16, text(tag)]));
    }
    if let Some(subscriptions) = &update.subscriptions {
        size = size.saturating_add(8);
        for sub in subscriptions {
            size = size.saturating_add(total([16, text(&sub.tag), subscription(&sub.kind)]));
        }
    }
    if let Some(command) = &update.presentation {
        size = size.saturating_add(match command {
            PresentationCommand::Open { panel, .. } => total([16, text(panel.as_str())]),
            PresentationCommand::Close => 8,
        });
    }
    size
}

/// Only the aggregate budget is checked here; the core validates individual items.
pub fn check_budget(update: &LogicUpdate, limits: &LogicLimits) -> Result<(), BudgetExceeded> {
    let actual = wire_size(update);
    if actual > limits.max_update_bytes {
        Err(BudgetExceeded {
            actual,
            limit: limits.max_update_bytes,
        })
    } else {
        Ok(())
    }
}

fn total(values: impl IntoIterator<Item = usize>) -> usize {
    values.into_iter().fold(0, usize::saturating_add)
}
fn text(s: &str) -> usize {
    8usize.saturating_add(s.len())
}
fn optional_text(s: &Option<String>) -> usize {
    s.as_deref().map_or(8, text)
}
fn scalar(value: &Scalar) -> usize {
    8usize.saturating_add(match value {
        Scalar::Boolean(_) => 1,
        Scalar::Number(_) => 8,
        Scalar::Text(value) => text(value),
    })
}
fn state_value(value: &StateValue) -> usize {
    match value {
        StateValue::Scalar(value) => scalar(value),
        StateValue::Records(records) => 8usize.saturating_add(total(records.iter().map(|r| {
            16usize.saturating_add(total(
                r.iter()
                    .map(|(key, value)| total([16, text(key), scalar(value)])),
            ))
        }))),
        StateValue::Canvas(commands) => 8usize.saturating_add(total(commands.iter().map(canvas))),
    }
}
fn canvas(command: &CanvasCommand) -> usize {
    64usize.saturating_add(match command {
        CanvasCommand::Text { text: value, .. } => text(value),
        CanvasCommand::Polyline { points, .. } => points.len().saturating_mul(8),
        CanvasCommand::StrokePath { segments, .. } | CanvasCommand::FillPath { segments, .. } => {
            segments.len().saturating_mul(32)
        }
        CanvasCommand::FillLinearGradientRect { .. } => 64,
        CanvasCommand::FillRect { .. }
        | CanvasCommand::Line { .. }
        | CanvasCommand::FillCircle { .. } => 0,
    })
}
fn spawn(spec: &SpawnSpec) -> usize {
    total([
        32,
        total(spec.argv.iter().map(|arg| total([16, text(arg)]))),
        total(
            spec.env
                .iter()
                .map(|(key, value)| total([16, text(key), text(value)])),
        ),
        optional_text(&spec.stdin),
    ])
}
fn dbus_value(value: &DbusValue) -> usize {
    total([16, text(&value.signature), text(&value.json)])
}
fn effect(value: &Effect) -> usize {
    32usize.saturating_add(match value {
        Effect::DesktopShortcut { keys, app } => total([
            total(keys.iter().map(|key| total([16, text(key)]))),
            optional_text(app),
        ]),
        Effect::DesktopWorkspace { id } | Effect::DesktopAppLaunch { id } => text(id),
        Effect::OpenUri { uri } => text(uri),
        Effect::SpawnRun(spec) => spawn(spec),
        Effect::DbusCall(call) => total([
            text(&call.destination),
            text(&call.path),
            text(&call.interface),
            text(&call.method),
            dbus_value(&call.args),
        ]),
        Effect::AudioSetVolume { .. }
        | Effect::AudioSetMute { .. }
        | Effect::MediaPlayPause
        | Effect::MediaNext
        | Effect::MediaPrevious
        | Effect::MediaSeek { .. }
        | Effect::DesktopBrightness { .. }
        | Effect::KeyTap { .. }
        | Effect::KeyDown { .. }
        | Effect::KeyUp { .. }
        | Effect::TimerAfter { .. } => 0,
    })
}
fn subscription(value: &SubscriptionKind) -> usize {
    32usize.saturating_add(match value {
        SubscriptionKind::DesktopFact { name } => text(&name.0),
        SubscriptionKind::SpawnLines(spec) => spawn(spec),
        SubscriptionKind::DbusSignal(spec) => total([
            optional_text(&spec.sender),
            optional_text(&spec.path),
            optional_text(&spec.interface),
            optional_text(&spec.member),
        ]),
        SubscriptionKind::DbusProperty(spec) => total([
            text(&spec.destination),
            text(&spec.path),
            text(&spec.interface),
            text(&spec.property),
        ]),
        SubscriptionKind::AudioWatch { .. }
        | SubscriptionKind::MediaWatch
        | SubscriptionKind::TimerInterval { .. } => 0,
    })
}

mod translation;

/// A conversion may discard individual values the model cannot represent.
#[derive(Clone, Debug, PartialEq)]
pub struct Translation<T> {
    pub value: T,
    pub warnings: Vec<String>,
}
