use crate::{CoreConfig, InstanceSnapshot, InstanceStatus, LogLevel, Output};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use touchbar_model::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InFlight {
    call: CallId,
    invocation: LogicCall,
    discard_update: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Instance {
    pub id: InstanceId,
    pub surface: SurfaceId,
    pub slot: SlotId,
    pub set: SetSpec,
    pub status: InstanceStatus,
    pub state: StatePatch,
    pub interactions: Vec<crate::WidgetInteraction>,
    pub retry_at: Option<Timestamp>,
    pub schema_fault: bool,
    wanted_visible: bool,
    runtime_alive: bool,
    failures: u32,
    fault: Option<String>,
    description: Option<LogicDescription>,
    queue: VecDeque<LogicCall>,
    in_flight: Option<InFlight>,
}
impl Instance {
    pub fn new(
        id: InstanceId,
        surface: SurfaceId,
        slot: SlotId,
        set: SetSpec,
        visible: bool,
    ) -> Self {
        Self {
            id,
            surface,
            slot,
            status: if set.logic.is_some() {
                InstanceStatus::Loading
            } else if visible {
                InstanceStatus::Visible
            } else {
                InstanceStatus::Hidden
            },
            runtime_alive: set.logic.is_some(),
            set,
            state: vec![],
            interactions: vec![],
            retry_at: None,
            schema_fault: false,
            wanted_visible: visible,
            failures: 0,
            fault: None,
            description: None,
            queue: VecDeque::new(),
            in_flight: None,
        }
    }
    pub fn snapshot(&self) -> InstanceSnapshot {
        InstanceSnapshot {
            id: self.id,
            surface: self.surface,
            set: self.set.reference.clone(),
            status: self.status,
            state: self.state.clone(),
            in_flight: self.in_flight.as_ref().map(|c| c.call),
            queue_len: self.queue.len(),
            retry_at: self.retry_at,
            fault: self.fault.clone(),
        }
    }
    pub fn creation(&self) -> Option<Output> {
        self.set
            .logic
            .as_ref()
            .map(|plugin| Output::CreateInstance {
                instance: self.id,
                surface: self.surface,
                plugin: plugin.clone(),
                set: self.set.reference.clone(),
            })
    }
    pub fn describe(
        &mut self,
        description: LogicDescription,
        persisted: StatePatch,
        now: Timestamp,
        config: &CoreConfig,
    ) -> Vec<Output> {
        if self.status != InstanceStatus::Loading
            || self.description.is_some()
            || !self.runtime_alive
        {
            return vec![];
        }
        if let Err(reason) = validate_description(&self.set, &description) {
            return self.fail(now, reason, false, config);
        }
        let persisted = persisted
            .into_iter()
            .filter(|(name, value)| {
                description
                    .state
                    .iter()
                    .any(|f| &f.name == name && f.persist && f.kind.accepts(value))
            })
            .collect();
        self.description = Some(description);
        self.queue.push_back(LogicCall::Init {
            config: self.set.logic_config.clone(),
            persisted,
        });
        vec![]
    }
    pub fn enqueue(&mut self, event: LogicEvent) {
        if self.set.logic.is_none()
            || !matches!(
                self.status,
                InstanceStatus::Hidden | InstanceStatus::Visible
            )
        {
            return;
        }
        if matches!(&event, LogicEvent::Action(_)) && self.status != InstanceStatus::Visible {
            return;
        }
        if let LogicEvent::Action(action) = &event
            && action.kind == ActionKind::ValueChanged
            && let Some(LogicCall::Handle(LogicEvent::Action(last))) = self.queue.back_mut()
            && last.kind == ActionKind::ValueChanged
            && last.action == action.action
            && last.record_key == action.record_key
        {
            *last = action.clone();
            return;
        }
        self.queue.push_back(LogicCall::Handle(event));
    }
    pub fn has_physical_work(&self) -> bool {
        let physical = |call: &LogicCall| {
            matches!(
                call,
                LogicCall::Handle(LogicEvent::Action(ActionEvent {
                    origin: Origin::Physical,
                    ..
                }))
            )
        };
        self.in_flight
            .as_ref()
            .is_some_and(|c| !c.discard_update && physical(&c.invocation))
            || self.queue.iter().any(physical)
    }
    pub fn discard_physical_work(&mut self) {
        let physical = |call: &LogicCall| {
            matches!(
                call,
                LogicCall::Handle(LogicEvent::Action(ActionEvent {
                    origin: Origin::Physical,
                    ..
                }))
            )
        };
        self.queue.retain(|call| !physical(call));
        if let Some(flight) = &mut self.in_flight
            && physical(&flight.invocation)
        {
            // Keep the slot occupied until completion; a preserved instance must
            // never execute a second wasm call concurrently with the abandoned one.
            flight.discard_update = true;
        }
    }
    pub fn dispatch(&mut self, next_call: &mut u64) -> Option<Output> {
        if self.in_flight.is_some() || !self.runtime_alive {
            return None;
        }
        let invocation = self.queue.pop_front()?;
        if self.status == InstanceStatus::Loading && !matches!(invocation, LogicCall::Init { .. }) {
            return None;
        }
        if matches!(
            self.status,
            InstanceStatus::Faulted | InstanceStatus::Disposed
        ) {
            return None;
        }
        let call = CallId(*next_call);
        *next_call = next_call.checked_add(1).expect("call id exhausted");
        self.in_flight = Some(InFlight {
            call,
            invocation: invocation.clone(),
            discard_update: false,
        });
        Some(Output::CallLogic {
            instance: self.id,
            call,
            invocation,
        })
    }
    pub fn update(
        &mut self,
        call: CallId,
        mut update: LogicUpdate,
        config: &CoreConfig,
        output: &mut Vec<Output>,
    ) -> Option<(LogicCall, LogicUpdate)> {
        if !self.matches_call(call) {
            return None;
        }
        let flight = self.in_flight.take().expect("matched call");
        if flight.discard_update {
            return None;
        }
        let invocation = flight.invocation;
        let description = self.description.as_ref().expect("described before calls");
        update.state.retain(|(name, value)| {
            let valid = description
                .state
                .iter()
                .any(|f| &f.name == name && f.kind.accepts(value))
                && value_within_limits(value, config);
            if !valid {
                output.push(self.warning(&format!("invalid state field {name}")));
            }
            valid
        });
        for (name, value) in &update.state {
            if let Some((_, existing)) = self.state.iter_mut().find(|(field, _)| field == name) {
                *existing = value.clone();
            } else {
                self.state.push((name.clone(), value.clone()));
            }
        }
        if matches!(invocation, LogicCall::Init { .. }) {
            self.status = InstanceStatus::Hidden;
            self.set_visible(self.wanted_visible);
        } else {
            self.failures = 0;
        }
        Some((invocation, update))
    }
    pub fn error(
        &mut self,
        call: CallId,
        message: String,
        now: Timestamp,
        config: &CoreConfig,
    ) -> Vec<Output> {
        if !self.matches_call(call) {
            return vec![];
        }
        if matches!(
            self.in_flight.as_ref().expect("matched call").invocation,
            LogicCall::Init { .. }
        ) {
            return self.fail(now, message, true, config);
        }
        self.in_flight = None;
        vec![self.warning(&message)]
    }
    pub fn call_fault(
        &mut self,
        call: CallId,
        reason: String,
        now: Timestamp,
        config: &CoreConfig,
    ) -> Vec<Output> {
        if !self.matches_call(call) {
            return vec![];
        }
        self.fail(now, reason, true, config)
    }
    pub fn load_fault(
        &mut self,
        reason: String,
        now: Timestamp,
        config: &CoreConfig,
    ) -> Vec<Output> {
        if self.status != InstanceStatus::Loading
            || self.description.is_some()
            || !self.runtime_alive
        {
            return vec![];
        }
        self.fail(now, reason, true, config)
    }
    pub fn set_visible(&mut self, visible: bool) {
        self.wanted_visible = visible;
        match (self.status, visible) {
            (InstanceStatus::Hidden, true) => {
                self.status = InstanceStatus::Visible;
                self.enqueue(LogicEvent::Shown);
            }
            (InstanceStatus::Visible, false) => {
                self.status = InstanceStatus::Hidden;
                self.queue
                    .retain(|c| !matches!(c, LogicCall::Handle(LogicEvent::Action(_))));
                self.enqueue(LogicEvent::Hidden);
            }
            _ => {}
        }
    }
    pub fn dispose(&mut self, next_call: &mut u64) -> Vec<Output> {
        self.set_visible(false);
        let mut output: Vec<_> = self.dispatch(next_call).into_iter().collect();
        if self.runtime_alive {
            output.push(Output::DisposeInstance { instance: self.id });
        }
        self.runtime_alive = false;
        self.status = InstanceStatus::Disposed;
        self.in_flight = None;
        self.queue.clear();
        self.retry_at = None;
        output
    }
    pub fn restart(&mut self) -> Option<Output> {
        if self.status != InstanceStatus::Faulted {
            return None;
        }
        if self.schema_fault {
            self.failures = 0;
        }
        self.id.generation = Generation(
            self.id
                .generation
                .0
                .checked_add(1)
                .expect("instance generation exhausted"),
        );
        self.status = InstanceStatus::Loading;
        self.runtime_alive = true;
        self.description = None;
        self.state.clear();
        self.queue.clear();
        self.in_flight = None;
        self.retry_at = None;
        self.schema_fault = false;
        self.fault = None;
        self.creation()
    }
    fn matches_call(&self, call: CallId) -> bool {
        self.in_flight.as_ref().is_some_and(|c| c.call == call)
            && matches!(
                self.status,
                InstanceStatus::Loading | InstanceStatus::Hidden | InstanceStatus::Visible
            )
    }
    fn warning(&self, message: &str) -> Output {
        Output::Log {
            level: LogLevel::Warn,
            event: format!(
                "instance {}/{}: {message}",
                self.id.number, self.id.generation.0
            ),
        }
    }
    fn fail(
        &mut self,
        now: Timestamp,
        reason: String,
        retryable: bool,
        config: &CoreConfig,
    ) -> Vec<Output> {
        let mut output = vec![self.warning(&reason)];
        if self.runtime_alive {
            output.push(Output::DisposeInstance { instance: self.id });
        }
        self.runtime_alive = false;
        self.status = InstanceStatus::Faulted;
        self.fault = Some(reason);
        self.schema_fault = !retryable;
        self.in_flight = None;
        self.queue.clear();
        self.retry_at = if retryable {
            let delay = config
                .backoff_initial_ms
                .saturating_mul(1_u64 << self.failures.min(63))
                .min(config.backoff_max_ms);
            self.failures = self.failures.saturating_add(1);
            Some(now.after(delay))
        } else {
            None
        };
        output
    }
}
fn validate_description(set: &SetSpec, description: &LogicDescription) -> Result<(), String> {
    let mut actions = BTreeSet::new();
    for (name, _) in &description.actions {
        if name.is_empty() || !actions.insert(name) {
            return Err("invalid or duplicate action name".into());
        }
    }
    let mut fields = BTreeSet::new();
    for field in &description.state {
        if field.name.is_empty() || !fields.insert(&field.name) {
            return Err("invalid or duplicate state field".into());
        }
        if let FieldKind::Records(record) = &field.kind {
            let mut names = BTreeSet::new();
            if record
                .iter()
                .any(|(name, _)| name.is_empty() || !names.insert(name))
            {
                return Err("invalid record schema".into());
            }
        }
    }
    for widget in &set.widgets {
        validate_widget(widget, description, None)?;
    }
    for panel in &set.panels {
        for widget in &panel.widgets {
            validate_widget(widget, description, None)?;
        }
    }
    if set.persist.iter().any(|name| {
        !description
            .state
            .iter()
            .any(|f| &f.name == name && f.persist)
    }) {
        return Err("persist field is missing or not declared persistent".into());
    }
    Ok(())
}
fn validate_widget(
    widget: &WidgetSpec,
    description: &LogicDescription,
    record: Option<&[(String, ScalarKind)]>,
) -> Result<(), String> {
    for (kind, name) in &widget.handlers {
        let Some((_, action)) = description
            .actions
            .iter()
            .find(|(action, _)| action == name)
        else {
            return Err(format!("unknown action: {name}"));
        };
        if !action.kinds.contains(kind) {
            return Err(format!("unknown action kind: {name}/{kind:?}"));
        }
        if (*kind == ActionKind::ValueChanged && action.value != Some(ScalarKind::Number))
            || matches!(action.value, Some(ScalarKind::Boolean | ScalarKind::Text))
        {
            return Err(format!("incompatible physical action value: {name}"));
        }
    }
    for (binding, name) in &widget.state_bindings {
        let scalar = match binding {
            StateBinding::Text | StateBinding::Symbol => Some(ScalarKind::Text),
            StateBinding::Value => Some(ScalarKind::Number),
            StateBinding::Enabled | StateBinding::Selected | StateBinding::Visible => {
                Some(ScalarKind::Boolean)
            }
            _ => None,
        };
        let valid = if let Some(field) = name.strip_prefix("record.") {
            scalar.is_some_and(|kind| {
                record.is_some_and(|fields| {
                    fields
                        .iter()
                        .any(|(name, actual)| name == field && *actual == kind)
                })
            })
        } else {
            description
                .state
                .iter()
                .find(|f| &f.name == name)
                .is_some_and(|field| match binding {
                    StateBinding::Records => matches!(field.kind, FieldKind::Records(_)),
                    StateBinding::Canvas => field.kind == FieldKind::Canvas,
                    _ => scalar.is_some_and(|kind| field.kind == FieldKind::Scalar(kind)),
                })
        };
        if !valid {
            return Err(format!("unknown or incompatible state field: {name}"));
        }
    }
    let child_record = widget
        .state_bindings
        .get(&StateBinding::Records)
        .and_then(|name| description.state.iter().find(|f| &f.name == name))
        .and_then(|field| match &field.kind {
            FieldKind::Records(fields) => Some(fields.as_slice()),
            _ => None,
        })
        .or(record);
    for child in &widget.children {
        validate_widget(child, description, child_record)?;
    }
    Ok(())
}
fn value_within_limits(value: &StateValue, config: &CoreConfig) -> bool {
    match value {
        StateValue::Scalar(Scalar::Text(text)) => text.len() <= config.max_text_bytes,
        StateValue::Records(rows) => {
            rows.len() <= config.max_records
                && rows.iter().all(|row| {
                    row.values()
                        .all(|v| !matches!(v, Scalar::Text(s) if s.len() > config.max_text_bytes))
                })
        }
        StateValue::Canvas(commands) => commands.iter().all(canvas_valid),
        _ => true,
    }
}
fn canvas_valid(command: &CanvasCommand) -> bool {
    fn point(p: &Point) -> bool {
        p.x.is_finite() && p.y.is_finite()
    }
    fn rect(r: &Rect) -> bool {
        point(&Point { x: r.x, y: r.y })
            && r.width.is_finite()
            && r.height.is_finite()
            && r.width >= 0.0
            && r.height >= 0.0
    }
    fn size(n: f32) -> bool {
        n.is_finite() && n >= 0.0
    }
    fn paint(p: &CanvasPaint) -> bool {
        (0.0..=1.0).contains(&p.opacity)
            && match p.color {
                CanvasColor::Role(_) => true,
                CanvasColor::Rgba {
                    red,
                    green,
                    blue,
                    alpha,
                } => [red, green, blue, alpha]
                    .iter()
                    .all(|n| (0.0..=1.0).contains(n)),
            }
    }
    fn path(segments: &[CanvasPathSegment]) -> bool {
        segments.iter().all(|s| match s {
            CanvasPathSegment::MoveTo(p) | CanvasPathSegment::LineTo(p) => point(p),
            CanvasPathSegment::QuadraticTo { control, endpoint } => {
                point(control) && point(endpoint)
            }
            CanvasPathSegment::CubicTo {
                control_one,
                control_two,
                endpoint,
            } => point(control_one) && point(control_two) && point(endpoint),
            CanvasPathSegment::Close => true,
        })
    }
    match command {
        CanvasCommand::FillRect {
            rect: r,
            radius,
            paint: p,
        } => rect(r) && size(*radius) && paint(p),
        CanvasCommand::Line {
            start,
            end,
            width,
            paint: p,
        } => point(start) && point(end) && size(*width) && paint(p),
        CanvasCommand::Polyline {
            points,
            width,
            paint: p,
        } => points.iter().all(point) && size(*width) && paint(p),
        CanvasCommand::FillCircle {
            center,
            radius,
            paint: p,
        } => point(center) && size(*radius) && paint(p),
        CanvasCommand::Text {
            rect: r,
            size: font,
            paint: p,
            ..
        } => rect(r) && size(*font) && paint(p),
        CanvasCommand::FillLinearGradientRect {
            rect: r,
            radius,
            gradient,
        } => {
            rect(r)
                && size(*radius)
                && point(&gradient.start)
                && point(&gradient.end)
                && paint(&gradient.start_color)
                && paint(&gradient.end_color)
        }
        CanvasCommand::StrokePath {
            segments,
            width,
            paint: p,
        } => path(segments) && size(*width) && paint(p),
        CanvasCommand::FillPath { segments, paint: p } => path(segments) && paint(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ready() -> (Instance, u64) {
        let mut set = SetSpec::default_for("p/slider".parse().unwrap());
        set.logic = Some("p".parse().unwrap());
        let mut instance = Instance::new(
            InstanceId {
                number: 1,
                generation: Generation(1),
            },
            SurfaceId(1),
            "main".parse().unwrap(),
            set,
            true,
        );
        let config = CoreConfig::default();
        instance.describe(LogicDescription::default(), vec![], Timestamp(0), &config);
        let mut next = 1;
        instance.dispatch(&mut next);
        instance.update(CallId(1), LogicUpdate::default(), &config, &mut vec![]);
        instance.dispatch(&mut next);
        instance.update(CallId(2), LogicUpdate::default(), &config, &mut vec![]);
        (instance, next)
    }
    fn value(value: f64) -> LogicEvent {
        LogicEvent::Action(ActionEvent {
            action: "slider".into(),
            kind: ActionKind::ValueChanged,
            value: Some(Scalar::Number(value)),
            record_key: None,
            origin: Origin::Physical,
            cancel_reason: None,
        })
    }
    fn payload(output: Option<Output>) -> LogicCall {
        let Some(Output::CallLogic { invocation, .. }) = output else {
            panic!("expected logic call");
        };
        invocation
    }
    #[test]
    fn coalescing_replaces_only_pending_values_and_keeps_running_value() {
        let (mut instance, mut next) = ready();
        let config = CoreConfig::default();
        instance.enqueue(value(0.1));
        assert_eq!(
            payload(instance.dispatch(&mut next)),
            LogicCall::Handle(value(0.1))
        );
        instance.enqueue(value(0.2));
        instance.enqueue(value(0.3));
        assert!(instance.dispatch(&mut next).is_none());
        instance.update(CallId(3), LogicUpdate::default(), &config, &mut vec![]);
        assert_eq!(
            payload(instance.dispatch(&mut next)),
            LogicCall::Handle(value(0.3))
        );
        assert!(instance.queue.is_empty());
    }
    #[test]
    fn hide_discards_pending_actions_but_completes_running_call_before_hidden() {
        let (mut instance, mut next) = ready();
        let config = CoreConfig::default();
        instance.enqueue(value(0.1));
        instance.dispatch(&mut next);
        instance.enqueue(value(0.2));
        instance.set_visible(false);
        instance.enqueue(value(0.3));
        assert!(instance.dispatch(&mut next).is_none());
        instance.update(CallId(3), LogicUpdate::default(), &config, &mut vec![]);
        assert_eq!(
            payload(instance.dispatch(&mut next)),
            LogicCall::Handle(LogicEvent::Hidden)
        );
        assert!(instance.queue.is_empty());
    }
    #[test]
    fn queue_restore_keeps_in_flight_correlation_and_pending_value() {
        let (mut instance, mut next) = ready();
        let config = CoreConfig::default();
        instance.enqueue(value(0.1));
        instance.dispatch(&mut next);
        instance.enqueue(value(0.9));
        let mut restored: Instance =
            serde_json::from_str(&serde_json::to_string(&instance).unwrap()).unwrap();
        assert!(restored.dispatch(&mut next).is_none());
        assert!(
            restored
                .update(CallId(999), LogicUpdate::default(), &config, &mut vec![])
                .is_none()
        );
        restored.update(CallId(3), LogicUpdate::default(), &config, &mut vec![]);
        assert_eq!(
            payload(restored.dispatch(&mut next)),
            LogicCall::Handle(value(0.9))
        );
    }
    #[test]
    fn activation_is_a_barrier_between_coalesced_value_changes() {
        let (mut instance, mut next) = ready();
        let config = CoreConfig::default();
        instance.enqueue(value(0.1));
        instance.dispatch(&mut next);
        instance.enqueue(value(0.2));
        let mut action = match value(0.3) {
            LogicEvent::Action(a) => a,
            _ => unreachable!(),
        };
        action.kind = ActionKind::Activated;
        instance.enqueue(LogicEvent::Action(action.clone()));
        instance.enqueue(value(0.4));
        for (call, expected) in [
            (3, value(0.2)),
            (4, LogicEvent::Action(action)),
            (5, value(0.4)),
        ] {
            instance.update(CallId(call), LogicUpdate::default(), &config, &mut vec![]);
            assert_eq!(
                payload(instance.dispatch(&mut next)),
                LogicCall::Handle(expected)
            );
        }
    }
    #[test]
    fn disposed_and_faulted_instances_never_dispatch_queued_actions() {
        for dispose in [false, true] {
            let (mut instance, mut next) = ready();
            instance.enqueue(value(0.1));
            instance.dispatch(&mut next);
            instance.enqueue(value(0.2));
            if dispose {
                instance.dispose(&mut next);
            } else {
                instance.call_fault(
                    CallId(3),
                    "trap".into(),
                    Timestamp(0),
                    &CoreConfig::default(),
                );
            }
            instance.enqueue(value(0.3));
            assert!(instance.dispatch(&mut next).is_none());
        }
    }
}
