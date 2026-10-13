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
    pub max_table_elements: usize,
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
            max_table_elements: 65_536,
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

/// Estimate inputs before allocating their WIT representation, using the same global budget.
pub fn wire_size_call(call: &LogicCall) -> usize {
    match call {
        LogicCall::Init { config, persisted } => wire_size_init(config, persisted),
        LogicCall::Handle(event) => wire_size_event(event),
    }
}
/// Full Init envelope, usable before constructing a call.
pub fn wire_size_init(config: &str, persisted: &StatePatch) -> usize {
    total([64, text(config), state_patch_size(persisted)])
}
/// Full Handle envelope, including tag and payload.
pub fn wire_size_event(event: &LogicEvent) -> usize {
    total([64, event_size(event)])
}
/// Worst-case spawn completion with max_output bytes in each stream.
pub fn wire_size_spawn_result(tag: &str, max_output: u32) -> usize {
    wire_size_event(&LogicEvent::EffectDone {
        tag: tag.into(),
        result: Ok(EffectResult::Spawn(SpawnResult {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
            truncated: false,
        })),
    })
    .saturating_add((max_output as usize).saturating_mul(2))
}
pub fn check_call_budget(call: &LogicCall, limits: &LogicLimits) -> Result<(), BudgetExceeded> {
    let actual = wire_size_call(call);
    if actual > limits.max_update_bytes {
        Err(BudgetExceeded {
            actual,
            limit: limits.max_update_bytes,
        })
    } else {
        Ok(())
    }
}
fn state_patch_size(patch: &StatePatch) -> usize {
    total(
        patch
            .iter()
            .map(|(k, v)| total([16, text(k), state_value(v)])),
    )
}
fn record_size(record: &RecordValue) -> usize {
    total(record.iter().map(|(k, v)| total([16, text(k), scalar(v)])))
}
fn fact_size(value: &FactValue) -> usize {
    total([
        16,
        match value {
            FactValue::Text(v) => text(v),
            FactValue::List(v) => total(v.iter().map(|v| total([16, text(v)]))),
            FactValue::Records(v) => total(v.iter().map(|v| total([16, record_size(v)]))),
            FactValue::Boolean(_) | FactValue::Number(_) => 8,
        },
    ])
}
fn error_size(error: &EffectError) -> usize {
    match error {
        EffectError::Failed(message) => total([16, text(message)]),
        _ => 16,
    }
}
fn event_size(event: &LogicEvent) -> usize {
    total([
        16,
        match event {
            LogicEvent::Action(action) => total([
                text(&action.action),
                action.value.as_ref().map_or(8, scalar),
                optional_text(&action.record_key),
                16,
            ]),
            LogicEvent::ContextChanged(ctx) => total([
                text(&ctx.surface_kind),
                text(&ctx.representation),
                text(&ctx.color_scheme),
                optional_text(&ctx.accent),
                text(&ctx.locale),
                8,
            ]),
            LogicEvent::PresentationClosed(panel) => text(panel.as_str()),
            LogicEvent::EffectDone { tag, result } => total([
                text(tag),
                match result {
                    Ok(EffectResult::Spawn(s)) => total([32, text(&s.stdout), text(&s.stderr)]),
                    Ok(EffectResult::Dbus(v)) => dbus_value(v),
                    Ok(EffectResult::None | EffectResult::Mute(_)) => 16,
                    Err(e) => error_size(e),
                },
            ]),
            LogicEvent::SubscriptionData { tag, data } => total([
                text(tag),
                match data {
                    SubscriptionData::Audio(s) => total([32, text(&s.name)]),
                    SubscriptionData::Media(s) => total([
                        64,
                        optional_text(&s.player),
                        text(&s.title),
                        text(&s.artist),
                        text(&s.album),
                        optional_text(&s.art_uri),
                    ]),
                    SubscriptionData::Fact(v) => v.as_ref().map_or(8, fact_size),
                    SubscriptionData::Line(v) => text(v),
                    SubscriptionData::Dbus(v) => dbus_value(v),
                    SubscriptionData::Tick => 8,
                },
            ]),
            LogicEvent::SubscriptionLost { tag, error } => total([text(tag), error_size(error)]),
            LogicEvent::Shown | LogicEvent::Hidden => 0,
        },
    ])
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
